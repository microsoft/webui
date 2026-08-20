// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Runtime-discovered progressive streaming hydration.

mod checkpoint;
mod error;
mod inventory;
mod owned;
mod root;
mod session;
mod state;
mod vm;

use serde_json::Value;

use crate::route_handler::Protocol;
use crate::{
    FlushWriter, HandlerError, RenderOptions, ResponseWriter, Result, WebUIHandler,
    WebUIProcessContext,
};

pub(crate) use error::streaming_boundary_error;
pub(crate) use inventory::{record_checkpoint_tag, streaming_template_already_sent};
pub use owned::{BufferSink, SessionOptions, StreamStep, StreamingSession};
pub(crate) use root::{
    consume_streaming_component_root, ensure_no_pending_streaming_root,
    prepare_generated_streaming_root, validate_pending_streaming_root,
    validate_streaming_root_opening, ComponentHostOrigin,
};
pub use session::{
    BoundaryDescriptor, BoundaryInstanceId, BoundaryKey, BoundaryMode, SpanInstanceId,
    StreamStatus, StreamingResponse, MAX_BOUNDARY_OCCURRENCES, MAX_CONTINUATION_DEPTH,
    MAX_KEYED_INSTANCES, MAX_OPEN_SPANS, MAX_SPAN_NESTING,
};
pub(crate) use state::StreamingRenderState;
pub(crate) use vm::PreparedContinuationStatePlan;

use root::process_streaming_root_signal;
use state::require_streaming_head_start;

pub(crate) const STREAMING_MARKER: &str = "<meta name=\"webui-streaming\" content=\"1\">";

/// Fixed-capacity scratch used to assemble short streaming markers and record
/// headers.
///
/// Every `ResponseWriter::write` is a dynamic call that reaches the transport,
/// so composing a marker byte-piece by byte-piece costs several indirections
/// per boundary. Formatting into this stack buffer collapses each marker or
/// record header into a single write.
pub(super) struct MarkerBuffer {
    bytes: [u8; Self::CAPACITY],
    len: usize,
}

impl MarkerBuffer {
    const CAPACITY: usize = 96;

    pub(super) const fn new() -> Self {
        Self {
            bytes: [0; Self::CAPACITY],
            len: 0,
        }
    }

    pub(super) fn push_str(&mut self, text: &str) -> Result<()> {
        let end = self
            .len
            .checked_add(text.len())
            .filter(|end| *end <= Self::CAPACITY)
            .ok_or_else(marker_overflow_error)?;
        self.bytes[self.len..end].copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }

    pub(super) fn push_usize(&mut self, mut value: usize) -> Result<()> {
        let mut digits = [0u8; 20];
        let mut offset = digits.len();
        loop {
            offset = offset.checked_sub(1).ok_or_else(marker_overflow_error)?;
            digits[offset] =
                b'0' + u8::try_from(value % 10).map_err(|_| marker_overflow_error())?;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        let end = self
            .len
            .checked_add(digits.len() - offset)
            .filter(|end| *end <= Self::CAPACITY)
            .ok_or_else(marker_overflow_error)?;
        self.bytes[self.len..end].copy_from_slice(&digits[offset..]);
        self.len = end;
        Ok(())
    }

    pub(super) fn flush_to<W: ResponseWriter + ?Sized>(&mut self, writer: &mut W) -> Result<()> {
        let text =
            std::str::from_utf8(&self.bytes[..self.len]).map_err(|_| marker_overflow_error())?;
        let result = writer.write(text);
        self.len = 0;
        result
    }
}

/// Write `<prefix><id>-->` with a single writer call.
pub(super) fn write_range_marker<W: ResponseWriter + ?Sized>(
    writer: &mut W,
    prefix: &str,
    id: u32,
) -> Result<()> {
    let mut buffer = MarkerBuffer::new();
    buffer.push_str(prefix)?;
    buffer.push_usize(usize::try_from(id).map_err(|_| marker_overflow_error())?)?;
    buffer.push_str("-->")?;
    buffer.flush_to(writer)
}

#[cold]
#[inline(never)]
fn marker_overflow_error() -> HandlerError {
    HandlerError::Invariant("streaming marker exceeded its fixed buffer".to_string())
}

struct StreamingSink<'w, W: FlushWriter + ?Sized> {
    transport: &'w mut W,
    component_opening: Option<ComponentOpening>,
    written: usize,
    flushed: usize,
}

struct ComponentOpening {
    bytes: String,
    root_offset: Option<usize>,
}

impl<W: FlushWriter + ?Sized> ResponseWriter for StreamingSink<'_, W> {
    fn write(&mut self, content: &str) -> Result<()> {
        if let Some(opening) = self.component_opening.as_mut() {
            opening.bytes.push_str(content);
            return Ok(());
        }
        self.written = self.written.wrapping_add(content.len());
        self.transport.write(content)
    }

    fn end(&mut self) -> Result<()> {
        self.transport.end()
    }

    fn stream_flush(&mut self) -> Result<()> {
        if self.component_opening.is_some() {
            return Err(HandlerError::Invariant(
                "streaming flush attempted while a component opening was buffered".to_string(),
            ));
        }
        // A flush at a position already delivered would cost a syscall without
        // releasing a byte, so semantic steps that produced no output between
        // two checkpoints collapse into the earlier flush.
        if self.written == self.flushed {
            return Ok(());
        }
        self.flushed = self.written;
        self.transport.flush()
    }

    fn stream_begin_component(&mut self) -> Result<()> {
        if self.component_opening.is_some() {
            return Err(HandlerError::Invariant(
                "nested component opening buffers are not valid".to_string(),
            ));
        }
        self.component_opening = Some(ComponentOpening {
            bytes: String::with_capacity(128),
            root_offset: None,
        });
        Ok(())
    }

    fn stream_mark_component_root(&mut self) -> Result<()> {
        let Some(opening) = self.component_opening.as_mut() else {
            return Err(HandlerError::Invariant(
                "component root marker has no buffered opening".to_string(),
            ));
        };
        if opening.root_offset.replace(opening.bytes.len()).is_some() {
            return Err(HandlerError::Invariant(
                "component opening contains duplicate root markers".to_string(),
            ));
        }
        Ok(())
    }

    fn stream_commit_component(
        &mut self,
        span_id: Option<u32>,
        enclosing_span_id: Option<u32>,
        deferred: bool,
    ) -> Result<()> {
        let Some(opening) = self.component_opening.take() else {
            return Err(HandlerError::Invariant(
                "component opening commit has no buffered bytes".to_string(),
            ));
        };
        let Some(root_offset) = opening.root_offset else {
            return Err(HandlerError::Invariant(
                "buffered component opening has no compiler root marker".to_string(),
            ));
        };
        if let Some(id) = span_id {
            write_range_marker(self, "<!--ws:", id)?;
        }
        self.write(&opening.bytes[..root_offset])?;
        if deferred {
            self.write(" data-ws")?;
            if let Some(id) = span_id {
                self.write(" data-ws-span=\"")?;
                write_u32(self, id)?;
                self.write("\"")?;
            }
            if let Some(id) = enclosing_span_id {
                self.write(" data-ws-enclosing=\"")?;
                write_u32(self, id)?;
                self.write("\"")?;
            }
        }
        self.write(&opening.bytes[root_offset..])
    }
}

fn write_u32<W: ResponseWriter + ?Sized>(writer: &mut W, mut value: u32) -> Result<()> {
    if value == 0 {
        return writer.write("0");
    }
    let mut digits = [0u8; 10];
    let mut offset = digits.len();
    while value != 0 {
        offset -= 1;
        let digit = u8::try_from(value % 10)
            .map_err(|_| HandlerError::Invariant("invalid decimal digit".to_string()))?;
        digits[offset] = b'0' + digit;
        value /= 10;
    }
    let value = std::str::from_utf8(&digits[offset..])
        .map_err(|_| HandlerError::Invariant("invalid decimal ID bytes".to_string()))?;
    writer.write(value)
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
                "structural signal arrived after body_end",
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

        if value.starts_with(root::STREAMING_ROOT_PREFIX) {
            process_streaming_root_signal(value, context)?;
            return Ok(true);
        }

        if value == "body_end" {
            require_streaming_head_start(context, "body_end")?;
            if context
                .streaming
                .as_ref()
                .is_some_and(|streaming| streaming.active_boundary.is_some())
            {
                return Err(streaming_boundary_error(
                    value,
                    "body ended while a boundary body was still rendering",
                ));
            }
            if let Some(html) = context.body_inject {
                context.writer.write(html)?;
            }
            if let Some(html) = context.state_inject.body_end {
                context.writer.write(html)?;
            }
            streaming_state(context)?.body_ended = true;
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

    /// Render every runtime boundary in document order as final.
    ///
    /// The value is snapshotted once when the response starts and every
    /// occurrence resumes against that snapshot, so a large state is projected
    /// once per response rather than re-merged once per boundary. Hosts that
    /// need asynchronous work — or genuinely new state — between occurrences
    /// should use [`Self::stream_response`] or [`StreamingSession`].
    pub fn render_streaming<'a, W: FlushWriter + ?Sized>(
        &self,
        protocol: &'a Protocol,
        state: &Value,
        options: &RenderOptions<'a>,
        writer: &'a mut W,
    ) -> Result<()> {
        let mut response = self.stream_response(protocol, options, writer)?;
        let mut status = response.start(state)?;
        while !status.done {
            let Some(boundary) = status.boundary.as_ref() else {
                return Err(HandlerError::Invariant(
                    "unfinished streaming step has no pending boundary".to_string(),
                ));
            };
            status = response.resume_current(boundary.instance_id, BoundaryMode::Final)?;
        }
        Ok(())
    }
}
