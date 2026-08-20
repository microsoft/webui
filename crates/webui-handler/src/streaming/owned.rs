// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Host-owned progressive sessions with pull-based byte delivery.

use std::sync::Arc;

use serde_json::Value;

use super::session::{
    BoundaryDescriptor, BoundaryInstanceId, BoundaryMode, SessionCall, SessionCore, StreamStatus,
};
use super::StreamingSink;
use crate::route_handler::Protocol;
use crate::{FlushWriter, RenderOptions, ResponseWriter, Result, WebUIHandler};

/// Reusable byte sink for host-owned sessions.
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

/// Owned per-response configuration.
#[derive(Clone, Debug)]
pub struct SessionOptions {
    /// Entry fragment to render.
    pub entry_id: String,
    /// Request path used for route matching.
    pub request_path: String,
    /// Optional CSP nonce.
    pub nonce: Option<String>,
    /// Optional trusted HTML injected at head_end.
    pub head_inject: Option<String>,
    /// Optional trusted HTML injected at body_end.
    pub body_inject: Option<String>,
}

impl SessionOptions {
    /// Create options for an entry and request path.
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

/// Bytes and continuation state returned by one owned semantic step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamStep {
    /// Complete bytes produced by this call.
    pub bytes: Vec<u8>,
    /// Next runtime occurrence waiting for resume.
    pub boundary: Option<BoundaryDescriptor>,
    /// True after terminal emission.
    pub done: bool,
}

/// A progressive response owned independently of any Rust borrow.
pub struct StreamingSession {
    handler: Arc<WebUIHandler>,
    protocol: Arc<Protocol>,
    options: SessionOptions,
    core: SessionCore,
    sink: BufferSink,
}

impl StreamingSession {
    /// Create an unstarted owned session.
    pub fn new(
        handler: Arc<WebUIHandler>,
        protocol: Arc<Protocol>,
        options: SessionOptions,
    ) -> Result<Self> {
        let core = SessionCore::new(&handler, &protocol, &options.entry_id)?;
        Ok(Self {
            handler,
            protocol,
            options,
            core,
            sink: BufferSink::default(),
        })
    }

    /// Render until the first runtime boundary occurrence or terminal.
    pub fn start(&mut self, state: &Value) -> Result<StreamStep> {
        self.sink.bytes.clear();
        let options = render_options(&self.options);
        let mut sink = StreamingSink {
            transport: &mut self.sink,
            component_opening: None,
            written: 0,
            flushed: 0,
        };
        let status = self.core.start(
            SessionCall {
                handler: &self.handler,
                protocol: &self.protocol,
                options: &options,
                writer: &mut sink,
            },
            state,
        );
        self.finish_step(status)
    }

    /// Commit the pending occurrence and advance.
    pub fn resume(
        &mut self,
        instance_id: BoundaryInstanceId,
        state: &Value,
        mode: BoundaryMode,
    ) -> Result<StreamStep> {
        self.sink.bytes.clear();
        let options = render_options(&self.options);
        let mut sink = StreamingSink {
            transport: &mut self.sink,
            component_opening: None,
            written: 0,
            flushed: 0,
        };
        let status = self.core.resume(
            SessionCall {
                handler: &self.handler,
                protocol: &self.protocol,
                options: &options,
                writer: &mut sink,
            },
            instance_id,
            state,
            mode,
        );
        self.finish_step(status)
    }

    /// Emit an update for one committed updatable runtime occurrence.
    pub fn update(&mut self, instance_id: BoundaryInstanceId, patch: &Value) -> Result<Vec<u8>> {
        self.sink.bytes.clear();
        let options = render_options(&self.options);
        let mut sink = StreamingSink {
            transport: &mut self.sink,
            component_opening: None,
            written: 0,
            flushed: 0,
        };
        let result = self.core.update(
            SessionCall {
                handler: &self.handler,
                protocol: &self.protocol,
                options: &options,
                writer: &mut sink,
            },
            instance_id,
            patch,
        );
        match result {
            Ok(()) => Ok(std::mem::take(&mut self.sink.bytes)),
            Err(error) => {
                self.sink.bytes.clear();
                Err(error)
            }
        }
    }

    /// Whether terminal emission completed.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.core.done
    }

    fn finish_step(&mut self, status: Result<StreamStatus>) -> Result<StreamStep> {
        match status {
            Ok(status) => Ok(StreamStep {
                bytes: std::mem::take(&mut self.sink.bytes),
                boundary: status.boundary,
                done: status.done,
            }),
            Err(error) => {
                self.sink.bytes.clear();
                Err(error)
            }
        }
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
