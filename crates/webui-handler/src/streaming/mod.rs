// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Progressive streaming hydration (`WebUIHandler::render_streaming`).
//!
//! An opt-in render mode in which each compile-time `<boundary>` commits an
//! independently hydratable island while the response is still open, instead
//! of deferring every island to one page-wide `#webui-data` block at
//! `body_end`. See DESIGN.md, "Progressive Streaming Hydration"
//! for the normative contract.
//!
//! The split mirrors the lifecycle of a single boundary:
//!
//! - [`state`] holds the request-local render state and the `head_start`
//!   precondition.
//! - [`root`] marks streamed SSR hosts so they defer until their boundary
//!   commits.
//! - [`inventory`] tracks what each checkpoint rendered and can reach.
//! - [`checkpoint`] writes the boundary envelope.
//! - [`error`] holds the cold diagnostic constructors.

mod checkpoint;
mod error;
mod inventory;
mod root;
mod state;

use std::borrow::Cow;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::route_handler::Protocol;
use crate::{
    FlushWriter, HandlerError, RenderOptions, ResponseWriter, Result, WebUIHandler,
    WebUIProcessContext, INITIAL_KEY_CAPACITY,
};

pub(crate) use error::streaming_boundary_error;
pub(crate) use inventory::{record_checkpoint_tag, streaming_template_already_sent};
pub(crate) use root::{
    consume_streaming_component_root, ensure_no_pending_streaming_root,
    prepare_generated_streaming_root, validate_pending_streaming_root,
    validate_streaming_root_opening, ComponentHostOrigin,
};
pub(crate) use state::{validate_streaming_head_start, StreamingRenderState};

use crate::write_usize;
use error::parse_boundary_sequence;
use root::process_streaming_root_signal;
use state::{
    increment_streaming_sequence, require_streaming_head_start, BOUNDARY_END_PREFIX,
    BOUNDARY_START_PREFIX,
};

pub(crate) const STREAMING_MARKER: &str = "<meta name=\"webui-streaming\" content=\"1\">";

/// Request-local streaming sink selected once at
/// [`WebUIHandler::render_streaming`] entry.
///
/// Wrapping the transport lets every write mark the current boundary dirty
/// while forwarding to the concrete writer with a static call: no per-write
/// `RefCell` borrow and no second virtual dispatch. `dirty` is a shared `Cell`
/// (plain load/store, no borrow check) that `body_end` reads to decide whether
/// trailing bytes need a final checkpoint.
struct StreamingSink<'w, W: FlushWriter + ?Sized> {
    transport: &'w mut W,
    dirty: &'w Cell<bool>,
}

impl<W: FlushWriter + ?Sized> ResponseWriter for StreamingSink<'_, W> {
    fn write(&mut self, content: &str) -> Result<()> {
        self.dirty.set(true);
        self.transport.write(content)
    }

    fn end(&mut self) -> Result<()> {
        self.transport.end()
    }

    fn stream_flush(&mut self) -> Result<()> {
        self.dirty.set(false);
        self.transport.flush()
    }
}

pub(super) fn streaming_state<'a, 'data, 'stream>(
    context: &'a mut WebUIProcessContext<'data, '_, 'stream>,
) -> Result<&'a mut StreamingRenderState<'data, 'stream>> {
    context.streaming.as_mut().ok_or_else(|| {
        HandlerError::Invariant("streaming signal processed outside a streaming render".to_string())
    })
}

pub(super) fn flush_streaming_transport(
    context: &mut WebUIProcessContext<'_, '_, '_>,
) -> Result<()> {
    if context.streaming.is_none() {
        return Err(HandlerError::Invariant(
            "streaming flush requested outside a streaming render".to_string(),
        ));
    }
    // The sink resets its own dirty flag as part of flushing, so there is no
    // separate `dirty.set(false)` here.
    context.writer.stream_flush()
}

impl WebUIHandler {
    pub(crate) fn process_streaming_signal<'data>(
        &self,
        value: &'data str,
        context: &mut WebUIProcessContext<'data, '_, '_>,
    ) -> Result<bool> {
        if context
            .streaming
            .as_ref()
            .is_some_and(|streaming| streaming.body_ended)
        {
            return Err(streaming_boundary_error(
                value,
                "structural signal arrived after the body_end terminal record",
            ));
        }

        if value == "head_start" {
            if context
                .streaming
                .as_ref()
                .is_some_and(|streaming| streaming.head_marker_emitted)
            {
                return Err(HandlerError::DuplicateStreamingHeadStart);
            }
            context.writer.write(STREAMING_MARKER)?;
            streaming_state(context)?.head_marker_emitted = true;
            return Ok(true);
        }

        if value.starts_with("streaming_root") {
            // Compiler-owned streamed SSR root marker: inject ` data-ws` inside
            // the component's opening tag (the parser placed this signal before
            // `>`). The host must live inside an open boundary, otherwise it can
            // never be activated — fail with a cold structured error.
            process_streaming_root_signal(value, context)?;
            return Ok(true);
        }
        if let Some(raw_sequence) = value.strip_prefix(BOUNDARY_START_PREFIX) {
            require_streaming_head_start(context, "streaming boundary")?;
            let sequence = parse_boundary_sequence(value, raw_sequence)?;
            let streaming = streaming_state(context)?;
            if let Some(active) = streaming.active_boundary {
                return Err(streaming_boundary_error(
                    value,
                    &format!("nested boundary {sequence}; boundary {active} is still open"),
                ));
            }
            if sequence != streaming.next_sequence {
                return Err(streaming_boundary_error(
                    value,
                    &format!(
                        "expected boundary sequence {}, received {sequence}",
                        streaming.next_sequence
                    ),
                ));
            }
            context.writer.write("<!--wb:")?;
            write_usize(context.writer, sequence)?;
            context.writer.write("-->")?;
            streaming_state(context)?.active_boundary = Some(sequence);
            return Ok(true);
        }
        if value.starts_with("boundary_start") {
            require_streaming_head_start(context, "streaming boundary")?;
            return Err(streaming_boundary_error(
                value,
                "expected `boundary_start:<decimal sequence>`",
            ));
        }

        if let Some(raw_sequence) = value.strip_prefix(BOUNDARY_END_PREFIX) {
            require_streaming_head_start(context, "streaming boundary")?;
            let sequence = parse_boundary_sequence(value, raw_sequence)?;
            let active = streaming_state(context)?.active_boundary.ok_or_else(|| {
                streaming_boundary_error(value, "boundary end has no matching start")
            })?;
            if active != sequence {
                return Err(streaming_boundary_error(
                    value,
                    &format!("boundary {active} is open, but boundary {sequence} ended"),
                ));
            }

            context.writer.write("<!--/wb:")?;
            write_usize(context.writer, sequence)?;
            context.writer.write("-->")?;
            self.emit_streaming_checkpoint(sequence, false, context)?;
            let streaming = streaming_state(context)?;
            streaming.active_boundary = None;
            increment_streaming_sequence(value, streaming)?;
            return Ok(true);
        }
        if value.starts_with("boundary_end") {
            require_streaming_head_start(context, "streaming boundary")?;
            return Err(streaming_boundary_error(
                value,
                "expected `boundary_end:<decimal sequence>`",
            ));
        }

        if value == "body_end" {
            require_streaming_head_start(context, "body_end")?;
            let (active_boundary, body_ended, needs_implicit, next_sequence) = {
                let streaming = streaming_state(context)?;
                (
                    streaming.active_boundary,
                    streaming.body_ended,
                    streaming.dirty.get() || !streaming.bootstrap_sent,
                    streaming.next_sequence,
                )
            };
            if let Some(active) = active_boundary {
                return Err(streaming_boundary_error(
                    value,
                    &format!("body ended while boundary {active} was still open"),
                ));
            }
            if body_ended {
                return Err(streaming_boundary_error(value, "duplicate body_end signal"));
            }
            if let Some(html) = context.body_inject {
                context.writer.write(html)?;
            }

            // Any raw/native tail bytes after the last commit require one
            // implicit final checkpoint. Registered component hosts cannot
            // appear outside an explicit boundary (the streaming_root branch
            // rejects them), so this tail never contains an untracked
            // interactive root. Coalesce the tail commit with the terminal:
            // emit a single terminal checkpoint (`terminal = true`) carrying the
            // tail bootstrap in one flush, rather than an implicit checkpoint
            // flush followed by a separate empty terminal flush. This may ship an
            // empty-state checkpoint for a scriptless tail, but never strands an
            // interactive root. When no tail exists, emit the standalone empty
            // terminal so the client still observes a markerless close.
            if needs_implicit {
                self.emit_streaming_checkpoint(next_sequence, true, context)?;
            } else {
                self.emit_streaming_terminal(next_sequence, context)?;
            }
            let streaming = streaming_state(context)?;
            streaming.body_ended = true;
            context.body_end_emitted = true;
            return Ok(true);
        }

        if value == "head_end" {
            require_streaming_head_start(context, "head_end")?;
        } else if value == "body_start" {
            require_streaming_head_start(context, "body_start")?;
        }

        Ok(false)
    }

    /// Render an opt-in progressive hydration response.
    ///
    /// Boundary signals are validated in document order and every committed
    /// checkpoint is flushed through `writer`. Unlike [`Self::render`], this
    /// mode emits boundary envelopes instead of a page-wide `#webui-data`
    /// block and requires structural `head_start` and `body_end` signals.
    pub fn render_streaming<'a, W: FlushWriter + ?Sized>(
        &self,
        protocol: &'a Protocol,
        state: &'a Value,
        options: &RenderOptions<'a>,
        writer: &mut W,
    ) -> Result<()> {
        let document = protocol.protocol();
        if !document.fragments.contains_key(options.entry_id) {
            return Err(HandlerError::MissingFragment(options.entry_id.to_string()));
        }
        validate_streaming_head_start(document, options.entry_id)?;

        let dirty = Cell::new(false);
        let inventory_bytes = protocol.component_index().len().div_ceil(8);
        // Select the streaming sink once, here at render entry. It borrows the
        // transport mutably and shares `dirty` with the render state; ordinary
        // per-write output never sees this wrapper.
        let mut sink = StreamingSink {
            transport: writer,
            dirty: &dirty,
        };
        let mut context = WebUIProcessContext {
            protocol: document,
            legacy_structural_signals: protocol.legacy_structural_signals(),
            state,
            writer: &mut sink,
            local_vars: HashMap::new(),
            component_attrs: HashMap::new(),
            request_path: options.request_path,
            route_base: Cow::Borrowed("/"),
            rendered_components: HashSet::new(),
            plugin: self.plugin_factory.map(|factory| factory()),
            route_children: Vec::new(),
            entry_id: options.entry_id,
            nonce: options.nonce.filter(|nonce| !nonce.is_empty()),
            head_inject: options.head_inject.filter(|html| !html.is_empty()),
            body_inject: options.body_inject.filter(|html| !html.is_empty()),
            head_end_emitted: false,
            component_index: protocol.component_index(),
            body_end_emitted: false,
            route_index: protocol.route_index(),
            route_chain_index: 0,
            streaming: Some(StreamingRenderState {
                dirty: &dirty,
                component_reachability: protocol.component_reachability(),
                head_marker_emitted: false,
                active_boundary: None,
                pending_root: None,
                generated_root_ready: false,
                next_sequence: 0,
                bootstrap_sent: false,
                body_ended: false,
                inventory: vec![0; inventory_bytes],
                inventory_delta: vec![0; inventory_bytes],
                inventory_hex: String::with_capacity(inventory_bytes * 2),
                template_inventory: vec![0; inventory_bytes],
                checkpoint_tags: Vec::new(),
                checkpoint_seen: vec![0; inventory_bytes],
                checkpoint_needs_expansion: false,
                state_key_scratch: Vec::with_capacity(INITIAL_KEY_CAPACITY),
                template_tag_scratch: Vec::new(),
                css_href_scratch: Vec::new(),
                style_spec_scratch: Vec::new(),
                reachability_stack: Vec::new(),
            }),
            json_scratch: Vec::new(),
            scope_pool: Vec::new(),
        };

        self.process_fragment_id(options.entry_id, &mut context)?;
        if !context
            .streaming
            .as_ref()
            .is_some_and(|streaming| streaming.body_ended)
        {
            return Err(HandlerError::MissingStreamingBodyEnd);
        }
        context.writer.end()
    }
}
