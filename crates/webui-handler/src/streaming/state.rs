// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Request-local state shared by the continuation VM and wire serializer.

use webui_protocol::WebUIProtocol;

use super::root::PendingStreamingRoot;
use super::streaming_boundary_error;
use crate::{route_handler, HandlerError, Result, WebUIProcessContext};

/// Capture buffers for one boundary or generated component span.
///
/// Boundary capture lives directly on [`StreamingRenderState`]. Open spans own
/// one of these records so nested spans never leak roots into an earlier
/// checkpoint. Buffers are swapped into the serializer and recycled after the
/// record commits.
pub(crate) struct RecordCapture {
    pub(super) tags: Vec<u32>,
    pub(super) walk_roots: Vec<(u32, Option<Box<str>>)>,
    pub(super) seen: Vec<u8>,
    pub(super) needs_expansion: bool,
}

impl RecordCapture {
    pub(crate) fn new(component_count: usize) -> Self {
        Self {
            tags: Vec::new(),
            walk_roots: Vec::new(),
            seen: vec![0; component_count.div_ceil(8)],
            needs_expansion: false,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.tags.clear();
        self.walk_roots.clear();
        self.seen.fill(0);
        self.needs_expansion = false;
    }
}

pub(crate) struct StreamingRenderState<'data> {
    pub(super) component_reachability: &'data route_handler::ComponentReachabilityIndex,
    pub(super) head_marker_emitted: bool,
    pub(super) active_boundary: Option<u32>,
    pub(super) current_span: Option<u32>,
    pub(super) pending_span_host: Option<u32>,
    pub(super) pending_root: Option<PendingStreamingRoot<'data>>,
    pub(super) generated_root_ready: bool,
    pub(super) next_record_sequence: usize,
    pub(super) bootstrap_sent: bool,
    pub(super) body_ended: bool,
    pub(super) inventory: Vec<u8>,
    pub(super) inventory_delta: Vec<u8>,
    pub(super) inventory_hex: String,
    /// Dedup bitset for template metadata and style closures already delivered.
    /// Kept separate from `inventory`: reachable-but-unrendered descendants
    /// receive metadata but must not be reported as rendered DOM.
    pub(super) template_inventory: Vec<u8>,
    /// Dedup bitset for style resource definitions already delivered. A resource
    /// can arrive transitively through an earlier root's closure before its own
    /// template or closure is emitted.
    pub(super) style_resource_inventory: Vec<u8>,
    /// Unique component indexes rendered since the previous checkpoint, in
    /// render order. Storing the startup-built index rather than the tag string
    /// keeps capture allocation-free, lets every consumer skip the
    /// `component_index` hash lookup it would otherwise repeat, and leaves this
    /// vector free of any borrow so a host-owned session can retain it across
    /// calls. The vector is cleared after commit while retaining capacity for
    /// the next checkpoint.
    pub(super) checkpoint_tags: Vec<u32>,
    pub(super) checkpoint_walk_roots: Vec<(u32, Option<Box<str>>)>,
    pub(super) checkpoint_seen: Vec<u8>,
    pub(super) checkpoint_needs_expansion: bool,
    /// Interned hydration key IDs for the record being committed.
    ///
    /// Integers instead of borrowed keys: the buffer outlives every semantic
    /// step in [`StreamingProgress`], so a checkpoint or update never allocates
    /// a fresh projection scratch.
    pub(super) state_key_ids: Vec<u32>,
    pub(super) template_tag_scratch: Vec<&'data str>,
    pub(super) css_href_scratch: Vec<&'data str>,
    pub(super) style_spec_scratch: Vec<&'data str>,
    pub(super) reachability_stack: Vec<u32>,
    pub(super) update_plans: Vec<Option<StateUpdatePlan>>,
}

pub(super) struct StateUpdatePlan {
    pub(super) requires_full_state: bool,
    pub(super) key_ids: Vec<u32>,
}

/// Owned state retained between calls by borrowed and host-owned sessions.
pub(crate) struct StreamingProgress {
    pub(super) head_marker_emitted: bool,
    pub(super) active_boundary: Option<u32>,
    pub(super) current_span: Option<u32>,
    pub(super) pending_span_host: Option<u32>,
    pub(super) generated_root_ready: bool,
    pub(super) next_record_sequence: usize,
    pub(super) bootstrap_sent: bool,
    pub(super) body_ended: bool,
    pub(super) inventory: Vec<u8>,
    pub(super) inventory_delta: Vec<u8>,
    pub(super) inventory_hex: String,
    pub(super) template_inventory: Vec<u8>,
    pub(super) style_resource_inventory: Vec<u8>,
    pub(super) checkpoint_tags: Vec<u32>,
    pub(super) checkpoint_walk_roots: Vec<(u32, Option<Box<str>>)>,
    pub(super) checkpoint_seen: Vec<u8>,
    pub(super) checkpoint_needs_expansion: bool,
    pub(super) state_key_ids: Vec<u32>,
    pub(super) reachability_stack: Vec<u32>,
    pub(super) update_plans: Vec<Option<StateUpdatePlan>>,
}

impl StreamingProgress {
    /// Allocate request-local bitsets for indexed components and style resources.
    pub(crate) fn new(component_count: usize, style_resource_count: usize) -> Self {
        let inventory_bytes = component_count.div_ceil(8);
        let style_inventory_bytes = style_resource_count.div_ceil(8);
        Self {
            head_marker_emitted: false,
            active_boundary: None,
            current_span: None,
            pending_span_host: None,
            generated_root_ready: false,
            next_record_sequence: 0,
            bootstrap_sent: false,
            body_ended: false,
            inventory: vec![0; inventory_bytes],
            inventory_delta: vec![0; inventory_bytes],
            inventory_hex: String::with_capacity(inventory_bytes * 2),
            template_inventory: vec![0; inventory_bytes],
            style_resource_inventory: vec![0; style_inventory_bytes],
            checkpoint_tags: Vec::new(),
            checkpoint_walk_roots: Vec::new(),
            checkpoint_seen: vec![0; inventory_bytes],
            checkpoint_needs_expansion: false,
            state_key_ids: Vec::new(),
            reachability_stack: Vec::new(),
            update_plans: Vec::new(),
        }
    }
}

impl<'data> StreamingRenderState<'data> {
    pub(crate) fn from_progress(
        progress: StreamingProgress,
        component_reachability: &'data route_handler::ComponentReachabilityIndex,
    ) -> Self {
        Self {
            component_reachability,
            pending_root: None,
            // Borrowed template/CSS scratch starts empty: only a record that
            // delivers first-time component metadata ever fills it, so a
            // steady-state step allocates nothing here.
            template_tag_scratch: Vec::new(),
            css_href_scratch: Vec::new(),
            style_spec_scratch: Vec::new(),
            head_marker_emitted: progress.head_marker_emitted,
            active_boundary: progress.active_boundary,
            current_span: progress.current_span,
            pending_span_host: progress.pending_span_host,
            generated_root_ready: progress.generated_root_ready,
            next_record_sequence: progress.next_record_sequence,
            bootstrap_sent: progress.bootstrap_sent,
            body_ended: progress.body_ended,
            inventory: progress.inventory,
            inventory_delta: progress.inventory_delta,
            inventory_hex: progress.inventory_hex,
            template_inventory: progress.template_inventory,
            style_resource_inventory: progress.style_resource_inventory,
            checkpoint_tags: progress.checkpoint_tags,
            checkpoint_walk_roots: progress.checkpoint_walk_roots,
            checkpoint_seen: progress.checkpoint_seen,
            checkpoint_needs_expansion: progress.checkpoint_needs_expansion,
            state_key_ids: progress.state_key_ids,
            reachability_stack: progress.reachability_stack,
            update_plans: progress.update_plans,
        }
    }

    pub(crate) fn into_progress(self) -> StreamingProgress {
        StreamingProgress {
            head_marker_emitted: self.head_marker_emitted,
            active_boundary: self.active_boundary,
            current_span: self.current_span,
            pending_span_host: self.pending_span_host,
            generated_root_ready: self.generated_root_ready,
            next_record_sequence: self.next_record_sequence,
            bootstrap_sent: self.bootstrap_sent,
            body_ended: self.body_ended,
            inventory: self.inventory,
            inventory_delta: self.inventory_delta,
            inventory_hex: self.inventory_hex,
            template_inventory: self.template_inventory,
            style_resource_inventory: self.style_resource_inventory,
            checkpoint_tags: self.checkpoint_tags,
            checkpoint_walk_roots: self.checkpoint_walk_roots,
            checkpoint_seen: self.checkpoint_seen,
            checkpoint_needs_expansion: self.checkpoint_needs_expansion,
            state_key_ids: self.state_key_ids,
            reachability_stack: self.reachability_stack,
            update_plans: self.update_plans,
        }
    }

    pub(crate) fn swap_capture(&mut self, capture: &mut RecordCapture) {
        std::mem::swap(&mut self.checkpoint_tags, &mut capture.tags);
        std::mem::swap(&mut self.checkpoint_walk_roots, &mut capture.walk_roots);
        std::mem::swap(&mut self.checkpoint_seen, &mut capture.seen);
        std::mem::swap(
            &mut self.checkpoint_needs_expansion,
            &mut capture.needs_expansion,
        );
    }
}

pub(super) fn require_streaming_head_start(
    context: &WebUIProcessContext<'_, '_, '_>,
    before: &'static str,
) -> Result<()> {
    if context
        .streaming
        .as_ref()
        .is_some_and(|streaming| streaming.head_marker_emitted)
    {
        Ok(())
    } else {
        Err(HandlerError::MissingStreamingHeadStart { before })
    }
}

pub(super) fn increment_streaming_record_sequence(
    signal: &str,
    streaming: &mut StreamingRenderState<'_>,
) -> Result<()> {
    streaming.next_record_sequence =
        streaming
            .next_record_sequence
            .checked_add(1)
            .ok_or_else(|| {
                streaming_boundary_error(signal, "record sequence overflowed the platform limit")
            })?;
    Ok(())
}

pub(crate) fn selected_state_snapshot(
    state: &serde_json::Value,
    keys: &[Box<str>],
) -> serde_json::Value {
    let serde_json::Value::Object(source) = state else {
        return serde_json::Value::Object(serde_json::Map::new());
    };
    let mut selected = serde_json::Map::with_capacity(keys.len());
    for key in keys {
        if let Some(value) = source.get(key.as_ref()) {
            selected.insert(key.to_string(), value.clone());
        }
    }
    serde_json::Value::Object(selected)
}

/// Merge the caller's state for this step into the retained continuation
/// snapshot.
///
/// Merging is *patch*, not replace: a key the caller omits keeps the value the
/// snapshot already holds, and no key is ever removed. Only the projected
/// surface is considered, so state a continuation never reads is not retained.
///
/// A value that is already identical is left alone, so a host resuming with the
/// same surface every step copies nothing — the comparison walks the shared
/// shape and stops at the first difference, while a copy would allocate a fresh
/// tree for data the snapshot already holds. Keys that do change reuse their
/// existing entry, letting [`serde_json::Value::clone_from`] reuse the previous
/// value's buffers.
pub(crate) fn overlay_selected_state(
    frozen: &mut serde_json::Value,
    state: &serde_json::Value,
    keys: &[Box<str>],
) {
    if !frozen.is_object() {
        *frozen = serde_json::Value::Object(serde_json::Map::new());
    }
    let serde_json::Value::Object(source) = state else {
        return;
    };
    let serde_json::Value::Object(target) = frozen else {
        return;
    };
    for key in keys {
        let Some(value) = source.get(key.as_ref()) else {
            continue;
        };
        match target.get_mut(key.as_ref()) {
            Some(slot) => {
                if slot != value {
                    slot.clone_from(value);
                }
            }
            None => {
                target.insert(key.to_string(), value.clone());
            }
        }
    }
}

/// Merge every top-level key of the caller's state into the retained snapshot.
///
/// Same patch semantics as [`overlay_selected_state`]: omitted keys keep their
/// snapshot value, nothing is removed, and an unchanged subtree is neither
/// copied nor reallocated.
pub(crate) fn overlay_full_state(frozen: &mut serde_json::Value, state: &serde_json::Value) {
    let serde_json::Value::Object(source) = state else {
        return;
    };
    if !frozen.is_object() {
        *frozen = serde_json::Value::Object(serde_json::Map::new());
    }
    let serde_json::Value::Object(target) = frozen else {
        return;
    };
    for (key, value) in source {
        match target.get_mut(key) {
            Some(slot) => {
                if slot != value {
                    slot.clone_from(value);
                }
            }
            None => {
                target.insert(key.clone(), value.clone());
            }
        }
    }
}

pub(crate) fn protocol_fragment<'a>(
    protocol: &'a WebUIProtocol,
    id: &str,
) -> Result<&'a webui_protocol::FragmentList> {
    protocol
        .fragments
        .get(id)
        .ok_or_else(|| HandlerError::MissingFragment(id.to_string()))
}
