// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Per-checkpoint component bookkeeping: what was rendered, what it can
//! reach, and whose metadata has already been delivered.
//!
//! All three are bitsets indexed by the startup-built component index, so a
//! commit is bit arithmetic over reused buffers rather than set allocation.

use std::collections::HashMap;

use super::error::component_style_inventory_error;
use super::StreamingRenderState;
use crate::{HandlerError, Result, WebUIProcessContext};

/// Record a rendered component tag into the current checkpoint's exact capture
/// set, deduplicated by the reusable bitset. Only the startup-built component
/// index is retained, so capture allocates nothing per tag and every consumer
/// avoids re-hashing the tag name. Tags without an inventory bit are ignored
/// (they carry no template or hydration state anyway).
pub(crate) fn record_checkpoint_tag(
    context: &mut WebUIProcessContext<'_, '_, '_>,
    fragment_id: &str,
) {
    let Some(&index) = context.component_index.get(fragment_id) else {
        return;
    };
    let route_dependent = context.streaming.as_ref().is_some_and(|streaming| {
        streaming.component_reachability.is_route_dependent(index) == Some(true)
    });
    let route_base = route_dependent.then(|| context.route_base.as_ref().into());
    let Some(streaming) = context.streaming.as_mut() else {
        return;
    };
    if let Some(route_base) = route_base {
        let route_base: Box<str> = route_base;
        let already_recorded = streaming.checkpoint_walk_roots.iter().any(|(root, base)| {
            *root == index
                && base
                    .as_ref()
                    .is_some_and(|base| base.as_ref() == route_base.as_ref())
        });
        if !already_recorded {
            if streaming.checkpoint_walk_roots.is_empty() {
                streaming
                    .checkpoint_walk_roots
                    .reserve(streaming.checkpoint_tags.len() + 1);
                streaming.checkpoint_walk_roots.extend(
                    streaming
                        .checkpoint_tags
                        .iter()
                        .copied()
                        .map(|root| (root, None)),
                );
            }
            streaming
                .checkpoint_walk_roots
                .push((index, Some(route_base)));
        }
    }
    let byte_index = (index / 8) as usize;
    let bit = 1u8 << (index % 8);
    if byte_index >= streaming.checkpoint_seen.len()
        || streaming.checkpoint_seen[byte_index] & bit != 0
    {
        return;
    }
    streaming.checkpoint_seen[byte_index] |= bit;
    streaming.checkpoint_tags.push(index);
    if !route_dependent && !streaming.checkpoint_walk_roots.is_empty() {
        streaming.checkpoint_walk_roots.push((index, None));
    }
    if streaming.component_reachability.requires_expansion(index) != Some(false) {
        streaming.checkpoint_needs_expansion = true;
    }
}

/// Commit the exact rendered tags to the cumulative DOM inventory and encode
/// this checkpoint's delta. Template delivery is tracked separately because a
/// reachable-but-unrendered descendant needs metadata without claiming live DOM.
pub(super) fn commit_checkpoint_inventory(streaming: &mut StreamingRenderState<'_>) -> Result<()> {
    streaming.inventory_delta.fill(0);
    for &index in &streaming.checkpoint_tags {
        let byte_index = usize::try_from(index / 8).map_err(|_| {
            HandlerError::Invariant("component inventory index does not fit usize".to_string())
        })?;
        if byte_index >= streaming.inventory.len() {
            return Err(HandlerError::Invariant(
                "component inventory index exceeds the request-local bitset".to_string(),
            ));
        }
        let bit = 1u8 << (index % 8);
        if streaming.inventory[byte_index] & bit == 0 {
            streaming.inventory[byte_index] |= bit;
            streaming.inventory_delta[byte_index] |= bit;
        }
    }

    const HEX: &[u8; 16] = b"0123456789abcdef";
    streaming.inventory_hex.clear();
    let byte_count = streaming
        .inventory_delta
        .iter()
        .rposition(|byte| *byte != 0)
        .map_or(0, |index| index + 1);
    for byte in &streaming.inventory_delta[..byte_count] {
        streaming
            .inventory_hex
            .push(char::from(HEX[usize::from(byte >> 4)]));
        streaming
            .inventory_hex
            .push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(())
}

/// Expand the exact rendered-root capture into its transitive component
/// surface using the startup-built integer dependency graph.
///
/// Returns `false` without mutating the capture when any root can reach an
/// authored route. Those uncommon surfaces require the request-aware fallback.
pub(super) fn expand_static_checkpoint_reachability(
    streaming: &mut StreamingRenderState<'_>,
) -> Result<bool> {
    if !streaming.checkpoint_needs_expansion {
        return Ok(true);
    }
    for &index in &streaming.checkpoint_tags {
        match streaming.component_reachability.is_route_dependent(index) {
            Some(false) => {}
            Some(true) => return Ok(false),
            None => {
                return Err(HandlerError::Invariant(
                    "component reachability index is incomplete".to_string(),
                ));
            }
        }
    }

    streaming.reachability_stack.clear();
    streaming
        .reachability_stack
        .extend(streaming.checkpoint_tags.iter().rev().copied());
    streaming.checkpoint_tags.clear();
    streaming.checkpoint_seen.fill(0);

    append_reachability_stack(streaming)?;

    Ok(true)
}

fn append_reachability_stack(streaming: &mut StreamingRenderState<'_>) -> Result<()> {
    while let Some(index) = streaming.reachability_stack.pop() {
        let byte_index = usize::try_from(index / 8).map_err(|_| {
            HandlerError::Invariant("component reachability index does not fit usize".to_string())
        })?;
        if byte_index >= streaming.checkpoint_seen.len() {
            return Err(HandlerError::Invariant(
                "component reachability index exceeds the request-local bitset".to_string(),
            ));
        }
        let bit = 1u8 << (index % 8);
        if streaming.checkpoint_seen[byte_index] & bit != 0 {
            continue;
        }
        streaming.checkpoint_seen[byte_index] |= bit;

        streaming.checkpoint_tags.push(index);
        let Some(dependencies) = streaming.component_reachability.dependencies(index) else {
            return Err(HandlerError::Invariant(
                "component reachability dependencies are missing".to_string(),
            ));
        };
        streaming
            .reachability_stack
            .extend(dependencies.iter().rev().copied());
    }

    Ok(())
}

pub(super) fn replace_checkpoint_reachability(
    streaming: &mut StreamingRenderState<'_>,
    component_index: &HashMap<String, u32>,
    reachable: &[String],
) {
    streaming.checkpoint_tags.clear();
    streaming.checkpoint_seen.fill(0);
    for name in reachable {
        let Some(&index) = component_index.get(name) else {
            continue;
        };
        let byte_index = (index / 8) as usize;
        let bit = 1u8 << (index % 8);
        if byte_index < streaming.checkpoint_seen.len()
            && streaming.checkpoint_seen[byte_index] & bit == 0
        {
            streaming.checkpoint_seen[byte_index] |= bit;
            streaming.checkpoint_tags.push(index);
        }
    }
}

/// Whether one component's template/CSS metadata has already been delivered by
/// an earlier checkpoint in this render.
///
/// A plain bitset probe: ordinary rendering emits a component's CSS importmap
/// inline on first render, so streaming must not emit it a second time for a
/// component whose metadata already rode along in a committed boundary.
///
/// Inlined because it is reached from `emit_css_module`, which runs once per
/// rendered component on the ordinary render path.
#[inline]
pub(crate) fn streaming_template_already_sent(
    streaming: &StreamingRenderState<'_>,
    component_index: &HashMap<String, u32>,
    tag: &str,
) -> bool {
    let Some(&index) = component_index.get(tag) else {
        return false;
    };
    let byte_index = (index / 8) as usize;
    byte_index < streaming.template_inventory.len()
        && streaming.template_inventory[byte_index] & (1u8 << (index % 8)) != 0
}

/// Mark one component's template/CSS metadata as delivered.
///
/// Takes the startup-built component index directly: the caller already holds
/// it from the checkpoint capture, so no tag hash is repeated here.
///
/// Returns `true` exactly once per indexed component for this render.
pub(super) fn mark_streaming_template_sent(
    streaming: &mut StreamingRenderState<'_>,
    index: u32,
) -> Result<bool> {
    let byte_index = usize::try_from(index / 8).map_err(|_| {
        HandlerError::Invariant("component template index does not fit usize".to_string())
    })?;
    if byte_index >= streaming.template_inventory.len() {
        return Err(HandlerError::Invariant(
            "component template index exceeds the request-local bitset".to_string(),
        ));
    }
    let bit = 1u8 << (index % 8);
    if streaming.template_inventory[byte_index] & bit != 0 {
        return Ok(false);
    }
    streaming.template_inventory[byte_index] |= bit;
    Ok(true)
}

/// Mark one indexed style resource definition as delivered.
pub(super) fn mark_streaming_style_resource_sent(
    streaming: &mut StreamingRenderState<'_>,
    index: u32,
) -> Result<()> {
    let byte_index = usize::try_from(index / 8)
        .map_err(|_| component_style_inventory_error("component style index does not fit usize"))?;
    if byte_index >= streaming.style_resource_inventory.len() {
        return Err(component_style_inventory_error(
            "component style index exceeds the request-local bitset",
        ));
    }
    streaming.style_resource_inventory[byte_index] |= 1u8 << (index % 8);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route_handler::Protocol;
    use std::collections::HashMap;
    use webui_protocol::{FragmentList, WebUIProtocol};

    #[test]
    fn checkpoint_tag_buffers_retain_capacity_across_commits() {
        // Deterministic capacity-reuse proof for the checkpoint tag vector /
        // dedup bitsets: rendered inventory and template delivery remain
        // independent, and the capture buffers keep their capacity once cleared.
        let protocol = Protocol::new(WebUIProtocol::with_tokens(
            HashMap::from([
                ("comp-a".to_string(), FragmentList::default()),
                ("comp-b".to_string(), FragmentList::default()),
            ]),
            Vec::new(),
        ));
        let mut streaming = StreamingRenderState::from_progress(
            super::super::state::StreamingProgress::new(2),
            protocol.component_reachability(),
        );
        streaming.head_marker_emitted = true;
        let index = protocol.component_index();
        let comp_a = index["comp-a"];
        let comp_b = index["comp-b"];

        // First checkpoint: both tags become rendered and have metadata sent.
        streaming.checkpoint_tags.push(comp_a);
        streaming.checkpoint_tags.push(comp_b);
        commit_checkpoint_inventory(&mut streaming).unwrap();
        assert_eq!(streaming.inventory_hex, "03");
        assert!(mark_streaming_template_sent(&mut streaming, comp_a).unwrap());
        assert!(mark_streaming_template_sent(&mut streaming, comp_b).unwrap());
        let capacity = streaming.checkpoint_tags.capacity();
        assert!(capacity >= 2);

        // Reset for the next checkpoint (mirrors the checkpoint tail).
        streaming.checkpoint_tags.clear();
        streaming.checkpoint_seen.fill(0);
        streaming.checkpoint_needs_expansion = false;

        // Second checkpoint: comp-a reused — no new metadata or DOM delta.
        streaming.checkpoint_tags.push(comp_a);
        commit_checkpoint_inventory(&mut streaming).unwrap();
        assert_eq!(streaming.inventory_hex, "");
        assert!(!mark_streaming_template_sent(&mut streaming, comp_a).unwrap());
        assert!(
            streaming.checkpoint_tags.capacity() >= capacity,
            "checkpoint tag buffer capacity must be reused, not reallocated"
        );
    }
}
