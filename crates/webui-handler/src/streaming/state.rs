// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Request-local state shared by the continuation VM and wire serializer.

use std::collections::HashSet;

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
    pub(super) fragment_source_refs: Vec<u32>,
    /// Membership index for `fragment_source_refs`, so retaining an input that
    /// a sibling call already retained costs one hash lookup instead of a scan
    /// of every ref this span accumulated.
    retained_sources: HashSet<u32>,
    /// This host's own props, sorted by name and held as immutable handles.
    ///
    /// A span closes after its boundary resumed with the host's next state, so
    /// the record would otherwise prime the host from caller state it never
    /// rendered with. Handles are `Arc` clones of the values the props already
    /// resolved to, so retaining them copies no JSON subtree.
    pub(super) owner_props: Vec<(Box<str>, crate::state_view::SharedValue)>,
    pub(super) tags: Vec<u32>,
    pub(super) walk_roots: Vec<(u32, Option<Box<str>>)>,
    pub(super) seen: Vec<u8>,
    pub(super) needs_expansion: bool,
}

impl RecordCapture {
    pub(crate) fn new(component_count: usize) -> Self {
        Self {
            fragment_source_refs: Vec::new(),
            retained_sources: HashSet::new(),
            owner_props: Vec::new(),
            tags: Vec::new(),
            walk_roots: Vec::new(),
            seen: vec![0; component_count.div_ceil(8)],
            needs_expansion: false,
        }
    }

    /// Retain `id` for this record's host, keeping first-use order and emitting
    /// each identifier exactly once.
    pub(super) fn retain_source(&mut self, id: u32) {
        if self.retained_sources.insert(id) {
            self.fragment_source_refs.push(id);
        }
    }

    pub(crate) fn clear(&mut self) {
        self.fragment_source_refs.clear();
        self.retained_sources.clear();
        self.owner_props.clear();
        self.tags.clear();
        self.walk_roots.clear();
        self.seen.fill(0);
        self.needs_expansion = false;
    }
}

pub(crate) struct StreamingRenderState<'data> {
    pub(super) fragment_sources: super::fragment_sources::FragmentSources,
    pub(super) checkpoint_source_refs: Vec<u32>,
    pub(super) component_reachability: &'data route_handler::ComponentReachabilityIndex,
    pub(super) head_marker_emitted: bool,
    pub(super) active_boundary: Option<u32>,
    pub(super) current_span: Option<u32>,
    pub(super) pending_span_host: Option<u32>,
    pub(super) pending_root: Option<PendingStreamingRoot<'data>>,
    pub(super) generated_root_ready: bool,
    pub(super) next_record_sequence: usize,
    /// Changes only when a host overlay changes the retained render snapshot.
    pub(super) state_revision: usize,
    /// Exact prior range record eligible as a browser state base.
    pub(super) last_state_record_sequence: Option<usize>,
    pub(super) last_state_revision: usize,
    pub(super) last_state_full: bool,
    pub(super) last_state_key_ids: Vec<u32>,
    /// Reusable `current - prior` projection scratch.
    pub(super) state_delta_key_ids: Vec<u32>,
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
    pub(super) fragment_sources: super::fragment_sources::FragmentSources,
    pub(super) checkpoint_source_refs: Vec<u32>,
    pub(super) head_marker_emitted: bool,
    pub(super) active_boundary: Option<u32>,
    pub(super) current_span: Option<u32>,
    pub(super) pending_span_host: Option<u32>,
    pub(super) generated_root_ready: bool,
    pub(super) next_record_sequence: usize,
    pub(super) state_revision: usize,
    pub(super) last_state_record_sequence: Option<usize>,
    pub(super) last_state_revision: usize,
    pub(super) last_state_full: bool,
    pub(super) last_state_key_ids: Vec<u32>,
    pub(super) state_delta_key_ids: Vec<u32>,
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
            fragment_sources: super::fragment_sources::FragmentSources::default(),
            checkpoint_source_refs: Vec::new(),
            head_marker_emitted: false,
            active_boundary: None,
            current_span: None,
            pending_span_host: None,
            generated_root_ready: false,
            next_record_sequence: 0,
            state_revision: 0,
            last_state_record_sequence: None,
            last_state_revision: 0,
            last_state_full: false,
            last_state_key_ids: Vec::new(),
            state_delta_key_ids: Vec::new(),
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
            fragment_sources: progress.fragment_sources,
            checkpoint_source_refs: progress.checkpoint_source_refs,
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
            state_revision: progress.state_revision,
            last_state_record_sequence: progress.last_state_record_sequence,
            last_state_revision: progress.last_state_revision,
            last_state_full: progress.last_state_full,
            last_state_key_ids: progress.last_state_key_ids,
            state_delta_key_ids: progress.state_delta_key_ids,
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
            fragment_sources: self.fragment_sources,
            checkpoint_source_refs: self.checkpoint_source_refs,
            head_marker_emitted: self.head_marker_emitted,
            active_boundary: self.active_boundary,
            current_span: self.current_span,
            pending_span_host: self.pending_span_host,
            generated_root_ready: self.generated_root_ready,
            next_record_sequence: self.next_record_sequence,
            state_revision: self.state_revision,
            last_state_record_sequence: self.last_state_record_sequence,
            last_state_revision: self.last_state_revision,
            last_state_full: self.last_state_full,
            last_state_key_ids: self.last_state_key_ids,
            state_delta_key_ids: self.state_delta_key_ids,
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
        std::mem::swap(
            &mut self.checkpoint_source_refs,
            &mut capture.fragment_source_refs,
        );
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
    sequence: &mut usize,
) -> Result<()> {
    *sequence = sequence.checked_add(1).ok_or_else(|| {
        streaming_boundary_error(signal, "record sequence overflowed the platform limit")
    })?;
    Ok(())
}

pub(crate) fn increment_state_revision(progress: &mut StreamingProgress) -> Result<()> {
    progress.state_revision = progress
        .state_revision
        .checked_add(1)
        .ok_or_else(state_revision_overflow_error)?;
    Ok(())
}

#[cold]
#[inline(never)]
fn state_revision_overflow_error() -> HandlerError {
    HandlerError::Invariant("streaming state revision overflowed the platform limit".to_string())
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
