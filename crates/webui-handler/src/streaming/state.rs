// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Request-local streaming render state and the head-start precondition.
//!
//! Every field here lives for exactly one response. The scratch vectors and
//! bitsets are reused across checkpoints so a boundary commit allocates
//! nothing on the steady-state path.

use webui_protocol::{web_ui_fragment::Fragment, WebUIProtocol};

use super::root::PendingStreamingRoot;
use super::streaming_boundary_error;
use crate::{route_handler, structural_signal_value, HandlerError, Result, WebUIProcessContext};

pub(super) const BOUNDARY_START_PREFIX: &str = "boundary_start:";
pub(super) const BOUNDARY_END_PREFIX: &str = "boundary_end:";

pub(crate) struct StreamingRenderState<'data> {
    /// Startup-built component dependency graph. Route-free checkpoints use its
    /// integer edges instead of walking/cloning protocol fragments per request.
    pub(super) component_reachability: &'data route_handler::ComponentReachabilityIndex,
    pub(super) head_marker_emitted: bool,
    pub(super) active_boundary: Option<usize>,
    /// Parser-produced root signal awaiting its exact `>`/component sequence.
    /// The tag is borrowed from the protocol signal, so validation allocates
    /// nothing on the successful per-component path.
    pub(super) pending_root: Option<PendingStreamingRoot<'data>>,
    /// Set only between a handler-generated route host's `data-ws` injection
    /// and its immediately following component render.
    pub(super) generated_root_ready: bool,
    /// Next compile-time boundary marker expected in the entry fragment.
    pub(super) next_boundary_id: usize,
    /// Next wire-record sequence. Updates consume record sequence numbers but
    /// do not affect compile-time boundary IDs.
    pub(super) next_record_sequence: usize,
    /// Selected by the host immediately before rendering the next boundary.
    pub(super) checkpoint_updatable: bool,
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
    /// Source-ordered roots for the request-aware checkpoint walk. `None` marks
    /// a route-independent root; route-dependent roots retain the base active at
    /// render time. The vector stays empty for route-free checkpoints and is
    /// backfilled lazily when their first route-dependent root appears. The
    /// route base is owned because this uncommon fallback already converts every
    /// root to an owned string, so borrowing bought nothing.
    pub(super) checkpoint_walk_roots: Vec<(u32, Option<Box<str>>)>,
    /// Dedup bitset (one bit per component index) marking tags already recorded
    /// into `checkpoint_tags` for the current checkpoint. Cleared and reused
    /// alongside `checkpoint_tags`.
    pub(super) checkpoint_seen: Vec<u8>,
    /// Set while capturing a root whose startup graph contains descendants or
    /// authored routes. Leaf-only checkpoints retain the original zero-walk path.
    pub(super) checkpoint_needs_expansion: bool,
    /// Reusable allowlist storage for checkpoint-local hydration state.
    pub(super) state_key_scratch: Vec<&'data str>,
    /// Component names for the current checkpoint's capture, resolved once from
    /// `checkpoint_tags` at commit. Borrowed from the startup reachability index
    /// and cleared before the commit returns, so it never spans a host call.
    pub(super) checkpoint_name_scratch: Vec<&'data str>,
    /// Reusable first-delivery metadata tag list.
    pub(super) template_tag_scratch: Vec<&'data str>,
    /// Reusable CSS bootstrap lists for newly delivered templates.
    pub(super) css_href_scratch: Vec<&'data str>,
    pub(super) style_spec_scratch: Vec<&'data str>,
    /// Reusable integer DFS stack for expanding route-free checkpoint surfaces.
    pub(super) reachability_stack: Vec<u32>,
    /// State projection retained only for boundaries explicitly committed as
    /// updatable. Final boundaries retain no update metadata. Keys are owned so
    /// the plan survives a host-owned session's call boundaries; the cost is one
    /// small allocation per key per updatable commit, never per rendered root.
    pub(super) update_plans: Vec<Option<StateUpdatePlan>>,
}

pub(super) struct StateUpdatePlan {
    pub(super) requires_full_state: bool,
    pub(super) keys: Vec<Box<str>>,
}

/// The half of [`StreamingRenderState`] that carries no borrow.
///
/// A host-owned response session parks this between calls and rebuilds the
/// borrowed half from its retained protocol. Splitting the state this way is
/// what lets a session outlive one `&Protocol` borrow without any self-
/// referential storage: everything that must survive a call boundary is owned,
/// and everything borrowed is either derivable from the protocol or provably
/// empty between calls.
pub(crate) struct StreamingProgress {
    pub(super) head_marker_emitted: bool,
    pub(super) active_boundary: Option<usize>,
    pub(super) generated_root_ready: bool,
    pub(super) next_boundary_id: usize,
    pub(super) next_record_sequence: usize,
    pub(super) checkpoint_updatable: bool,
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
    pub(super) reachability_stack: Vec<u32>,
    pub(super) update_plans: Vec<Option<StateUpdatePlan>>,
}

impl StreamingProgress {
    /// Allocate the request-local bitsets for a protocol with `component_count`
    /// indexed components.
    pub(crate) fn new(component_count: usize) -> Self {
        let inventory_bytes = component_count.div_ceil(8);
        Self {
            head_marker_emitted: false,
            active_boundary: None,
            generated_root_ready: false,
            next_boundary_id: 0,
            next_record_sequence: 0,
            checkpoint_updatable: false,
            bootstrap_sent: false,
            body_ended: false,
            inventory: vec![0; inventory_bytes],
            inventory_delta: vec![0; inventory_bytes],
            inventory_hex: String::with_capacity(inventory_bytes * 2),
            template_inventory: vec![0; inventory_bytes],
            style_resource_inventory: vec![0; inventory_bytes],
            checkpoint_tags: Vec::new(),
            checkpoint_walk_roots: Vec::new(),
            checkpoint_seen: vec![0; inventory_bytes],
            checkpoint_needs_expansion: false,
            reachability_stack: Vec::new(),
            update_plans: Vec::new(),
        }
    }
}

impl<'data> StreamingRenderState<'data> {
    /// Rebuild the borrowed half around parked progress.
    ///
    /// The scratch vectors start empty because every one of them is cleared at
    /// the end of the commit that fills it, so no borrowed value is ever live
    /// across a host call boundary.
    pub(crate) fn from_progress(
        progress: StreamingProgress,
        component_reachability: &'data route_handler::ComponentReachabilityIndex,
    ) -> Self {
        Self {
            component_reachability,
            pending_root: None,
            state_key_scratch: Vec::with_capacity(crate::INITIAL_KEY_CAPACITY),
            checkpoint_name_scratch: Vec::new(),
            template_tag_scratch: Vec::new(),
            css_href_scratch: Vec::new(),
            style_spec_scratch: Vec::new(),
            head_marker_emitted: progress.head_marker_emitted,
            active_boundary: progress.active_boundary,
            generated_root_ready: progress.generated_root_ready,
            next_boundary_id: progress.next_boundary_id,
            next_record_sequence: progress.next_record_sequence,
            checkpoint_updatable: progress.checkpoint_updatable,
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
            reachability_stack: progress.reachability_stack,
            update_plans: progress.update_plans,
        }
    }

    /// Park the owned half, dropping every borrow.
    pub(crate) fn into_progress(self) -> StreamingProgress {
        StreamingProgress {
            head_marker_emitted: self.head_marker_emitted,
            active_boundary: self.active_boundary,
            generated_root_ready: self.generated_root_ready,
            next_boundary_id: self.next_boundary_id,
            next_record_sequence: self.next_record_sequence,
            checkpoint_updatable: self.checkpoint_updatable,
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
            reachability_stack: self.reachability_stack,
            update_plans: self.update_plans,
        }
    }
}

pub(crate) fn validate_streaming_head_start(
    protocol: &WebUIProtocol,
    entry_id: &str,
) -> Result<()> {
    let fragments = protocol
        .fragments
        .get(entry_id)
        .ok_or_else(|| HandlerError::MissingFragment(entry_id.to_string()))?;
    for fragment in &fragments.fragments {
        let Some(Fragment::Signal(signal)) = fragment.fragment.as_ref() else {
            continue;
        };
        let Some(value) = structural_signal_value(signal) else {
            continue;
        };
        if value == "head_start" {
            return Ok(());
        }
        let before = match value {
            "head_end" => Some("head_end"),
            "body_start" => Some("body_start"),
            "body_end" => Some("body_end"),
            value if value.starts_with("boundary_start") || value.starts_with("boundary_end") => {
                Some("streaming boundary")
            }
            _ => None,
        };
        if let Some(before) = before {
            return Err(HandlerError::MissingStreamingHeadStart { before });
        }
    }
    Err(HandlerError::MissingStreamingHeadStart {
        before: "end of entry fragment",
    })
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

pub(super) fn increment_streaming_boundary_id(
    signal: &str,
    streaming: &mut StreamingRenderState<'_>,
) -> Result<()> {
    streaming.next_boundary_id = streaming.next_boundary_id.checked_add(1).ok_or_else(|| {
        streaming_boundary_error(signal, "boundary sequence overflowed the platform limit")
    })?;
    Ok(())
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
