// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Request-local streaming render state and the head-start precondition.
//!
//! Every field here lives for exactly one response. The scratch vectors and
//! bitsets are reused across checkpoints so a boundary commit allocates
//! nothing on the steady-state path.

use std::borrow::Cow;

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
    pub(super) next_sequence: usize,
    pub(super) bootstrap_sent: bool,
    pub(super) body_ended: bool,
    pub(super) inventory: Vec<u8>,
    pub(super) inventory_delta: Vec<u8>,
    pub(super) inventory_hex: String,
    /// Dedup bitset for template/CSS metadata already delivered. Kept separate
    /// from `inventory`: reachable-but-unrendered descendants receive metadata
    /// but must not be reported as rendered DOM.
    pub(super) template_inventory: Vec<u8>,
    /// Unique component tags rendered since the previous checkpoint, in render
    /// order. Borrowed from `component_index` keys (`&'data str`) so capture is
    /// allocation-free per tag. The vector is cleared after commit while retaining
    /// capacity for the next checkpoint.
    pub(super) checkpoint_tags: Vec<&'data str>,
    /// Route-dependent rendered roots paired with the route base active at
    /// render time. This cold-path vector stays empty for route-free surfaces;
    /// relative-route expansion cannot reconstruct these bases at commit time.
    pub(super) checkpoint_route_roots: Vec<(&'data str, Cow<'data, str>)>,
    /// Dedup bitset (one bit per component index) marking tags already recorded
    /// into `checkpoint_tags` for the current checkpoint. Cleared and reused
    /// alongside `checkpoint_tags`.
    pub(super) checkpoint_seen: Vec<u8>,
    /// Set while capturing a root whose startup graph contains descendants or
    /// authored routes. Leaf-only checkpoints retain the original zero-walk path.
    pub(super) checkpoint_needs_expansion: bool,
    /// Reusable allowlist storage for checkpoint-local hydration state.
    pub(super) state_key_scratch: Vec<&'data str>,
    /// Reusable first-delivery metadata tag list.
    pub(super) template_tag_scratch: Vec<&'data str>,
    /// Reusable CSS bootstrap lists for newly delivered templates.
    pub(super) css_href_scratch: Vec<&'data str>,
    pub(super) style_spec_scratch: Vec<&'data str>,
    /// Reusable integer DFS stack for expanding route-free checkpoint surfaces.
    pub(super) reachability_stack: Vec<u32>,
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
    context: &WebUIProcessContext<'_, '_>,
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

pub(super) fn increment_streaming_sequence(
    signal: &str,
    streaming: &mut StreamingRenderState<'_>,
) -> Result<()> {
    streaming.next_sequence = streaming.next_sequence.checked_add(1).ok_or_else(|| {
        streaming_boundary_error(signal, "boundary sequence overflowed the platform limit")
    })?;
    Ok(())
}
