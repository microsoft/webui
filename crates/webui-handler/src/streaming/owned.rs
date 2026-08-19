// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Host-owned streaming sessions for bindings that cannot hold a Rust borrow.
//!
//! [`StreamingResponse`](super::StreamingResponse) borrows its handler,
//! protocol, options, and transport for the life of one response. That is the
//! right shape for a Rust host, which keeps the response on its own stack, but
//! it cannot cross a foreign-function boundary: a Node class instance, a C
//! opaque pointer, or a .NET `SafeHandle` must stay alive between calls that
//! Rust never sees.
//!
//! [`StreamingSession`] closes that gap without self-referential storage. It
//! retains the handler and protocol behind `Arc`, parks the response's owned
//! progress between calls, and rebuilds the borrowed half for the duration of
//! each call. Nothing borrowed ever spans a call boundary, so this is ordinary
//! safe Rust.
//!
//! It also inverts transport ownership. Each method returns the bytes it
//! produced instead of writing them, so the host writes to its own socket and
//! applies its own backpressure — the thing a push-based chunk callback cannot
//! express.

use std::sync::Arc;

use serde_json::Value;

use super::session::{BoundaryId, BoundaryMode, ParkedResponse};
use super::StreamingResponse;
use crate::route_handler::Protocol;
use crate::{FlushWriter, RenderOptions, ResponseWriter, Result, WebUIHandler};

/// Collects response bytes for a host that owns its own transport.
///
/// `flush` is deliberately a no-op: a semantic flush means "these bytes are
/// complete and may be sent", and for an encoder that is exactly the point at
/// which the method returns them to the host.
#[derive(Default)]
pub struct BufferSink {
    bytes: Vec<u8>,
}

impl ResponseWriter for BufferSink {
    fn write(&mut self, content: &str) -> Result<()> {
        self.bytes.extend_from_slice(content.as_bytes());
        Ok(())
    }

    fn end(&mut self) -> Result<()> {
        Ok(())
    }
}

impl FlushWriter for BufferSink {
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Owned per-response configuration, so a session outlives its caller's strings.
#[derive(Clone, Debug)]
pub struct SessionOptions {
    /// Entry fragment to render.
    pub entry_id: String,
    /// Request path used for route matching.
    pub request_path: String,
    /// Optional CSP nonce for generated inline `<script>` tags.
    pub nonce: Option<String>,
    /// Optional HTML injected at the structural `head_end` boundary.
    pub head_inject: Option<String>,
    /// Optional HTML injected at the structural `body_end` boundary.
    pub body_inject: Option<String>,
}

impl SessionOptions {
    /// Create options for an entry fragment and request path.
    #[must_use]
    pub fn new(entry_id: impl Into<String>, request_path: impl Into<String>) -> Self {
        Self {
            entry_id: entry_id.into(),
            request_path: request_path.into(),
            nonce: None,
            head_inject: None,
            body_inject: None,
        }
    }
}

/// A progressive HTML response a foreign host drives one call at a time.
///
/// Every method returns the bytes that call produced. The host writes them to
/// its transport and decides when to continue, so backpressure and interleaved
/// async work stay under host control.
///
/// Ordering rules match [`StreamingResponse`]: the shell first, then each
/// boundary exactly once in declaration order, updates only after the target
/// boundary commits, and `finish` last. Violations return an actionable error
/// rather than corrupting the stream, and any render or transport failure
/// permanently poisons the session because bytes may already have been sent.
pub struct StreamingSession {
    handler: Arc<WebUIHandler>,
    protocol: Arc<Protocol>,
    options: SessionOptions,
    /// `None` only while a call is in flight, or once `finish` consumed the
    /// response either successfully or fatally.
    parked: Option<ParkedResponse>,
    sink: BufferSink,
    boundary_count: usize,
    /// Set only when a terminal record actually reached the transport.
    finished: bool,
}

impl StreamingSession {
    /// Start a host-driven progressive response.
    ///
    /// # Errors
    ///
    /// Returns an error when the entry is missing or its compiled streaming
    /// structure is malformed.
    pub fn new(
        handler: Arc<WebUIHandler>,
        protocol: Arc<Protocol>,
        options: SessionOptions,
    ) -> Result<Self> {
        let mut sink = BufferSink::default();
        let (parked, boundary_count) = {
            let render_options = render_options(&options);
            let response = handler.stream_response(&protocol, &render_options, &mut sink)?;
            let boundary_count = response.boundary_count();
            (response.park(), boundary_count)
        };
        Ok(Self {
            handler,
            protocol,
            options,
            parked: Some(parked),
            sink,
            boundary_count,
            finished: false,
        })
    }

    /// Resolve a free-form authored boundary name to an integer handle.
    ///
    /// Resolve once and reuse the handle; hot calls never hash a name.
    ///
    /// # Errors
    ///
    /// Returns an actionable error listing valid names when the entry does not
    /// declare `name`.
    pub fn boundary(&self, name: &str) -> Result<BoundaryId> {
        let names = self
            .protocol
            .streaming_boundary_names(&self.options.entry_id);
        names
            .iter()
            .position(|candidate| candidate == name)
            .ok_or_else(|| super::error::unknown_boundary_name_error(name, names))
            .and_then(BoundaryId::from_index)
    }

    /// Number of compile-time boundaries in this entry.
    #[must_use]
    pub fn boundary_count(&self) -> usize {
        self.boundary_count
    }

    /// Whether the response has emitted its terminal record.
    ///
    /// Stays `false` after a rejected or failed `finish`, because no terminal
    /// record reached the transport in either case.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Render the document prefix before the first boundary.
    ///
    /// # Errors
    ///
    /// Returns an error when called out of order or when rendering fails.
    pub fn write_shell(&mut self, state: &Value) -> Result<Vec<u8>> {
        self.run(|response| response.write_shell(state))
    }

    /// Render and commit the next compile-time boundary.
    ///
    /// # Errors
    ///
    /// Returns an error when boundaries are written out of declaration order or
    /// when rendering fails.
    pub fn write_boundary(
        &mut self,
        boundary: BoundaryId,
        state: &Value,
        mode: BoundaryMode,
    ) -> Result<Vec<u8>> {
        self.run(|response| response.write_boundary(boundary, state, mode))
    }

    /// Push a projected state patch to an already committed updatable boundary.
    ///
    /// # Errors
    ///
    /// Returns an error when the boundary has not committed, was committed as
    /// final, or when `state` is not a JSON object.
    pub fn update(&mut self, boundary: BoundaryId, state: &Value) -> Result<Vec<u8>> {
        self.run(|response| response.update(boundary, state))
    }

    /// Render the document tail and emit the terminal record.
    ///
    /// The session is finished afterwards and every later call fails.
    ///
    /// # Errors
    ///
    /// Returns an error when boundaries remain uncommitted or rendering fails.
    pub fn finish(&mut self, state: &Value) -> Result<Vec<u8>> {
        let parked = self.take_parked("finish")?;
        let result = {
            let Self {
                handler,
                protocol,
                options,
                sink,
                parked: slot,
                ..
            } = self;
            let render_options = render_options(options);
            match StreamingResponse::unpark(parked, handler, protocol, &render_options, sink) {
                // Ordering violations are rejected before any byte is written,
                // so park the response again and let the host commit what is
                // still outstanding instead of losing the open response.
                Ok(response) => match response.ensure_finishable() {
                    Ok(()) => response.finish(state),
                    Err(error) => {
                        *slot = Some(response.park());
                        Err(error)
                    }
                },
                Err(error) => Err(error),
            }
        };
        match result {
            Ok(()) => {
                self.finished = true;
                Ok(std::mem::take(&mut self.sink.bytes))
            }
            Err(error) => {
                self.sink.bytes = Vec::new();
                Err(error)
            }
        }
    }

    fn take_parked(&mut self, operation: &str) -> Result<ParkedResponse> {
        if let Some(parked) = self.parked.take() {
            return Ok(parked);
        }
        let reason = if self.finished {
            "the streaming response has already finished"
        } else {
            "the streaming response failed while emitting its terminal record and \
             cannot continue; bytes may already have been sent"
        };
        Err(super::error::boundary_order_error(operation, reason))
    }

    /// Rebuild the borrowed response, run one operation, and park it again.
    fn run(
        &mut self,
        operation: impl FnOnce(&mut StreamingResponse<'_, BufferSink>) -> Result<()>,
    ) -> Result<Vec<u8>> {
        let parked = self.take_parked("streaming operation")?;
        let Self {
            handler,
            protocol,
            options,
            sink,
            parked: slot,
            ..
        } = self;
        let render_options = render_options(options);
        let mut response =
            StreamingResponse::unpark(parked, handler, protocol, &render_options, sink)?;
        let result = operation(&mut response);
        *slot = Some(response.park());
        result?;
        Ok(std::mem::take(&mut self.sink.bytes))
    }
}

fn render_options(options: &SessionOptions) -> RenderOptions<'_> {
    RenderOptions {
        entry_id: &options.entry_id,
        request_path: &options.request_path,
        nonce: options.nonce.as_deref(),
        head_inject: options.head_inject.as_deref(),
        body_inject: options.body_inject.as_deref(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::StreamingSession;
    use crate::WebUIHandler;

    #[test]
    fn owned_streaming_session_supports_synchronized_hosts() {
        fn assert_send<T: Send>() {}
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send::<StreamingSession>();
        assert_send_sync::<Mutex<StreamingSession>>();
        assert_send_sync::<Arc<WebUIHandler>>();
    }
}
