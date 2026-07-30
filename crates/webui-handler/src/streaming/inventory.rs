// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Per-checkpoint component bookkeeping: what was rendered, what it can
//! reach, and whose metadata has already been delivered.
//!
//! All three are bitsets indexed by the startup-built component index, so a
//! commit is bit arithmetic over reused buffers rather than set allocation.

use std::collections::HashMap;

use super::StreamingRenderState;
use crate::{HandlerError, Result, WebUIProcessContext};

/// Record a rendered component tag into the current checkpoint's exact capture
/// set, deduplicated by the reusable bitset. The tag is borrowed from
/// `component_index` (`&'data str`) so capture allocates nothing per tag. Tags
/// without an inventory bit are ignored (they carry no template or hydration
/// state anyway).
pub(crate) fn record_checkpoint_tag<'data>(
    context: &mut WebUIProcessContext<'data, '_, '_>,
    fragment_id: &str,
) {
    // Copy the `'data` index reference out first so the borrowed key outlives the
    // subsequent mutable borrow of `context.streaming` (disjoint fields).
    let index_map: &'data HashMap<String, u32> = context.component_index;
    let Some((tag, &index)) = index_map.get_key_value(fragment_id) else {
        return;
    };
    let tag: &'data str = tag.as_str();
    let route_dependent = context.streaming.as_ref().is_some_and(|streaming| {
        streaming.component_reachability.is_route_dependent(index) == Some(true)
    });
    let route_base = route_dependent.then(|| context.route_base.clone());
    let Some(streaming) = context.streaming.as_mut() else {
        return;
    };
    if let Some(route_base) = route_base {
        let already_recorded = streaming.checkpoint_walk_roots.iter().any(|(root, base)| {
            *root == tag
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
                .push((tag, Some(route_base)));
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
    streaming.checkpoint_tags.push(tag);
    if !route_dependent && !streaming.checkpoint_walk_roots.is_empty() {
        streaming.checkpoint_walk_roots.push((tag, None));
    }
    if streaming.component_reachability.requires_expansion(index) != Some(false) {
        streaming.checkpoint_needs_expansion = true;
    }
}

/// Commit the exact rendered tags to the cumulative DOM inventory and encode
/// this checkpoint's delta. Template delivery is tracked separately because a
/// reachable-but-unrendered descendant needs metadata without claiming live DOM.
pub(super) fn commit_checkpoint_inventory(
    streaming: &mut StreamingRenderState<'_>,
    component_index: &HashMap<String, u32>,
) -> Result<()> {
    streaming.inventory_delta.fill(0);
    for &tag in &streaming.checkpoint_tags {
        let Some(&index) = component_index.get(tag) else {
            continue;
        };
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
pub(super) fn expand_static_checkpoint_reachability<'data>(
    streaming: &mut StreamingRenderState<'data>,
    component_index: &HashMap<String, u32>,
) -> Result<bool> {
    if !streaming.checkpoint_needs_expansion {
        return Ok(true);
    }
    for &tag in &streaming.checkpoint_tags {
        let Some(&index) = component_index.get(tag) else {
            continue;
        };
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
    for &tag in streaming.checkpoint_tags.iter().rev() {
        if let Some(&index) = component_index.get(tag) {
            streaming.reachability_stack.push(index);
        }
    }
    streaming.checkpoint_tags.clear();
    streaming.checkpoint_seen.fill(0);

    append_reachability_stack(streaming)?;

    Ok(true)
}

fn append_reachability_stack<'data>(streaming: &mut StreamingRenderState<'data>) -> Result<()> {
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

        let Some(name) = streaming.component_reachability.name(index) else {
            return Err(HandlerError::Invariant(
                "component reachability name is missing".to_string(),
            ));
        };
        streaming.checkpoint_tags.push(name);
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

pub(super) fn replace_checkpoint_reachability<'data>(
    streaming: &mut StreamingRenderState<'data>,
    component_index: &'data HashMap<String, u32>,
    reachable: &[String],
) {
    streaming.checkpoint_tags.clear();
    streaming.checkpoint_seen.fill(0);
    for name in reachable {
        let Some((tag, &index)) = component_index.get_key_value(name) else {
            continue;
        };
        let byte_index = (index / 8) as usize;
        let bit = 1u8 << (index % 8);
        if byte_index < streaming.checkpoint_seen.len()
            && streaming.checkpoint_seen[byte_index] & bit == 0
        {
            streaming.checkpoint_seen[byte_index] |= bit;
            streaming.checkpoint_tags.push(tag.as_str());
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
/// Returns `true` exactly once per indexed component for this render.
pub(super) fn mark_streaming_template_sent(
    streaming: &mut StreamingRenderState<'_>,
    component_index: &HashMap<String, u32>,
    tag: &str,
) -> Result<bool> {
    let Some(&index) = component_index.get(tag) else {
        return Ok(false);
    };
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
        let mut streaming = StreamingRenderState {
            component_reachability: protocol.component_reachability(),
            head_marker_emitted: true,
            active_boundary: None,
            pending_root: None,
            generated_root_ready: false,
            next_boundary_id: 0,
            next_record_sequence: 0,
            checkpoint_updatable: false,
            bootstrap_sent: false,
            body_ended: false,
            inventory: vec![0u8; 1],
            inventory_delta: vec![0u8; 1],
            inventory_hex: String::new(),
            template_inventory: vec![0u8; 1],
            checkpoint_tags: Vec::new(),
            checkpoint_walk_roots: Vec::new(),
            checkpoint_seen: vec![0u8; 1],
            checkpoint_needs_expansion: false,
            state_key_scratch: Vec::new(),
            template_tag_scratch: Vec::new(),
            css_href_scratch: Vec::new(),
            style_spec_scratch: Vec::new(),
            reachability_stack: Vec::new(),
            update_plans: Vec::new(),
        };
        let index = protocol.component_index();

        // First checkpoint: both tags become rendered and have metadata sent.
        streaming.checkpoint_tags.push("comp-a");
        streaming.checkpoint_tags.push("comp-b");
        commit_checkpoint_inventory(&mut streaming, index).unwrap();
        assert_eq!(streaming.inventory_hex, "03");
        assert!(mark_streaming_template_sent(&mut streaming, index, "comp-a").unwrap());
        assert!(mark_streaming_template_sent(&mut streaming, index, "comp-b").unwrap());
        let capacity = streaming.checkpoint_tags.capacity();
        assert!(capacity >= 2);

        // Reset for the next checkpoint (mirrors the checkpoint tail).
        streaming.checkpoint_tags.clear();
        streaming.checkpoint_seen.fill(0);
        streaming.checkpoint_needs_expansion = false;

        // Second checkpoint: comp-a reused — no new metadata or DOM delta.
        streaming.checkpoint_tags.push("comp-a");
        commit_checkpoint_inventory(&mut streaming, index).unwrap();
        assert_eq!(streaming.inventory_hex, "");
        assert!(!mark_streaming_template_sent(&mut streaming, index, "comp-a").unwrap());
        assert!(
            streaming.checkpoint_tags.capacity() >= capacity,
            "checkpoint tag buffer capacity must be reused, not reallocated"
        );
    }
}
