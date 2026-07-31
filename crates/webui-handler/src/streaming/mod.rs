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
mod owned;
mod plan;
mod root;
mod session;
mod state;

use serde_json::Value;

use crate::route_handler::Protocol;
use crate::{
    FlushWriter, HandlerError, RenderOptions, ResponseWriter, Result, WebUIHandler,
    WebUIProcessContext,
};

pub(crate) use error::streaming_boundary_error;
pub(crate) use inventory::{record_checkpoint_tag, streaming_template_already_sent};
pub use owned::{BufferSink, SessionOptions, StreamingSession};
pub(crate) use plan::{PreparedStreamingEntryPlan, StreamingEntryPlan};
pub(crate) use root::{
    consume_streaming_component_root, ensure_no_pending_streaming_root,
    prepare_generated_streaming_root, validate_pending_streaming_root,
    validate_streaming_root_opening, ComponentHostOrigin,
};
pub use session::{BoundaryId, BoundaryMode, StreamingResponse};
pub(crate) use state::{validate_streaming_head_start, StreamingRenderState};

use crate::write_usize;
use error::parse_boundary_sequence;
use root::process_streaming_root_signal;
use state::{
    increment_streaming_boundary_id, increment_streaming_record_sequence,
    require_streaming_head_start, BOUNDARY_END_PREFIX, BOUNDARY_START_PREFIX,
};

pub(crate) const STREAMING_MARKER: &str = "<meta name=\"webui-streaming\" content=\"1\">";

/// Request-local streaming sink selected once at
/// [`WebUIHandler::render_streaming`] entry.
///
/// Wrapping the transport maps the internal streaming flush hook to the
/// caller's concrete [`FlushWriter`] with no second virtual dispatch.
struct StreamingSink<'w, W: FlushWriter + ?Sized> {
    transport: &'w mut W,
}

impl<W: FlushWriter + ?Sized> ResponseWriter for StreamingSink<'_, W> {
    fn write(&mut self, content: &str) -> Result<()> {
        self.transport.write(content)
    }

    fn end(&mut self) -> Result<()> {
        self.transport.end()
    }

    fn stream_flush(&mut self) -> Result<()> {
        self.transport.flush()
    }
}

pub(super) fn streaming_state<'a, 'data>(
    context: &'a mut WebUIProcessContext<'data, '_, '_>,
) -> Result<&'a mut StreamingRenderState<'data>> {
    context.streaming.as_deref_mut().ok_or_else(|| {
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
            if sequence != streaming.next_boundary_id {
                return Err(streaming_boundary_error(
                    value,
                    &format!(
                        "expected boundary sequence {}, received {sequence}",
                        streaming.next_boundary_id
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
            let (record_sequence, updatable) = {
                let streaming = streaming_state(context)?;
                (
                    streaming.next_record_sequence,
                    streaming.checkpoint_updatable,
                )
            };
            self.emit_streaming_checkpoint(record_sequence, sequence, updatable, context)?;
            let streaming = streaming_state(context)?;
            streaming.active_boundary = None;
            streaming.checkpoint_updatable = false;
            increment_streaming_boundary_id(value, streaming)?;
            increment_streaming_record_sequence(value, streaming)?;
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
            let (active_boundary, body_ended, next_record_sequence) = {
                let streaming = streaming_state(context)?;
                (
                    streaming.active_boundary,
                    streaming.body_ended,
                    streaming.next_record_sequence,
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

            // The terminal flush commits any raw/native tail bytes. Registered
            // component hosts cannot appear outside an explicit boundary, so a
            // rootless tail never needs another template or state projection.
            // Keeping the envelope empty also prevents whitespace or a body
            // injection from re-sending complete state on legacy protocols.
            self.emit_streaming_terminal(next_record_sequence, context)?;
            let streaming = streaming_state(context)?;
            streaming.body_ended = true;
            increment_streaming_record_sequence(value, streaming)?;
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
        state: &Value,
        options: &RenderOptions<'a>,
        writer: &'a mut W,
    ) -> Result<()> {
        let mut response = self.stream_response(protocol, options, writer)?;
        response.write_shell_buffered(state)?;
        let boundary_count = response.boundary_count();
        for boundary in 0..boundary_count {
            let id = BoundaryId::from_index(boundary)?;
            response.write_boundary(id, state, BoundaryMode::Final)?;
        }
        response.finish(state)
    }
}
