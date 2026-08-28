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
///
/// Records where the streaming transport flushed so a session can prove each
/// returned step ends on a real flush boundary rather than silently merging a
/// checkpoint with the bytes that follow it.
#[derive(Default)]
pub struct BufferSink {
    bytes: Vec<u8>,
    last_flush: usize,
}

impl BufferSink {
    fn reset(&mut self) {
        self.bytes.clear();
        self.last_flush = 0;
    }
}

impl ResponseWriter for BufferSink {
    fn write(&mut self, content: &str) -> Result<()> {
        self.bytes.extend_from_slice(content.as_bytes());
        Ok(())
    }

    fn write_attribute(&mut self, name: &str, value: &str) -> Result<()> {
        crate::append_attribute_to_bytes(&mut self.bytes, name, value);
        Ok(())
    }

    fn write_boolean_attribute(&mut self, name: &str) -> Result<()> {
        crate::append_boolean_attribute_to_bytes(&mut self.bytes, name);
        Ok(())
    }

    fn end(&mut self) -> Result<()> {
        Ok(())
    }
}

impl FlushWriter for BufferSink {
    fn flush(&mut self) -> Result<()> {
        self.last_flush = self.bytes.len();
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
///
/// The bytes are one semantic write segment ending on a transport flush
/// boundary: the shell prefix, exactly one committed occurrence, the parent
/// bytes between two occurrences, or the tail plus terminal. A host writes them
/// with a single `write` + `flush`.
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
///
/// Exposes the same step machine as [`super::StreamingResponse`]:
/// [`Self::start`] writes the shell prefix, [`Self::resume`] writes exactly one
/// occurrence through its checkpoint, and [`Self::advance`] writes the parent
/// bytes that follow it.
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
        self.step(|core, call| core.start(call, state))
    }

    /// Render to the first occurrence by moving caller-owned state into the
    /// continuation snapshot.
    ///
    /// Async hosts that freshly decode or load state should prefer this method:
    /// a full-state continuation takes ownership of the value without cloning
    /// it, while a keyed continuation moves only its selected top-level values.
    pub fn start_owned(&mut self, state: Value) -> Result<StreamStep> {
        self.step(move |core, call| core.start_owned(call, state))
    }

    /// Commit the pending occurrence through its checkpoint, then stop.
    ///
    /// The returned bytes hold that occurrence's record and nothing that
    /// follows it. Call [`Self::advance`] for the parent bytes.
    ///
    /// [`BoundaryMode::Updatable`] is refused once the response has committed
    /// as many updatable occurrences as the browser retains. The refusal
    /// produces no bytes and leaves the occurrence pending, so it can be
    /// committed with [`BoundaryMode::Final`] instead.
    pub fn resume(
        &mut self,
        instance_id: BoundaryInstanceId,
        state: &Value,
        mode: BoundaryMode,
    ) -> Result<StreamStep> {
        self.step(|core, call| core.resume(call, instance_id, state, mode))
    }

    /// Commit the pending occurrence by moving caller-owned state into the
    /// retained continuation snapshot, then stop at its checkpoint flush.
    ///
    /// This is the preferred async-host path when the state was freshly decoded
    /// or loaded for this occurrence. Moving changed top-level values avoids
    /// cloning or deeply comparing their subtrees while preserving the same
    /// patch semantics as [`Self::resume`].
    pub fn resume_owned(
        &mut self,
        instance_id: BoundaryInstanceId,
        state: Value,
        mode: BoundaryMode,
    ) -> Result<StreamStep> {
        self.step(move |core, call| core.resume_owned(call, instance_id, state, mode))
    }

    /// Commit the pending occurrence against the session's retained state.
    ///
    /// The call still returns immediately after the checkpoint flush, preserving
    /// the host's async pause point before [`Self::advance`], but avoids a state
    /// overlay when no data changed since the preceding step.
    pub fn resume_current(
        &mut self,
        instance_id: BoundaryInstanceId,
        mode: BoundaryMode,
    ) -> Result<StreamStep> {
        self.step(|core, call| core.resume_current(call, instance_id, mode))
    }

    /// Write the ordinary parent bytes that follow a committed occurrence.
    ///
    /// Valid only after [`Self::resume`].
    pub fn advance(&mut self) -> Result<StreamStep> {
        self.step(SessionCore::advance)
    }

    /// Emit an update for one committed updatable runtime occurrence.
    ///
    /// Valid between [`Self::resume`] and [`Self::advance`], so a host can
    /// revise the occurrence it just committed while the response stays open.
    pub fn update(&mut self, instance_id: BoundaryInstanceId, patch: &Value) -> Result<Vec<u8>> {
        self.sink.reset();
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
            Ok(()) => {
                verify_flush_boundary(&self.sink)?;
                Ok(std::mem::take(&mut self.sink.bytes))
            }
            Err(error) => {
                self.sink.reset();
                Err(error)
            }
        }
    }

    /// Whether terminal emission completed.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.core.done
    }

    fn step(
        &mut self,
        operation: impl FnOnce(&mut SessionCore, SessionCall<'_, '_>) -> Result<StreamStatus>,
    ) -> Result<StreamStep> {
        self.sink.reset();
        let options = render_options(&self.options);
        let mut sink = StreamingSink {
            transport: &mut self.sink,
            component_opening: None,
            written: 0,
            flushed: 0,
        };
        let status = operation(
            &mut self.core,
            SessionCall {
                handler: &self.handler,
                protocol: &self.protocol,
                options: &options,
                writer: &mut sink,
            },
        );
        match status {
            Ok(status) => {
                verify_flush_boundary(&self.sink)?;
                Ok(StreamStep {
                    bytes: std::mem::take(&mut self.sink.bytes),
                    boundary: status.boundary,
                    done: status.done,
                })
            }
            Err(error) => {
                self.sink.reset();
                Err(error)
            }
        }
    }
}

/// Reject a step whose buffered bytes extend past the last transport flush.
///
/// A host-owned session hands the caller a byte buffer instead of a live
/// transport, so a step that produced bytes after its checkpoint flushed would
/// silently merge two semantic segments into one host write. The check is one
/// integer compare per step.
fn verify_flush_boundary(sink: &BufferSink) -> Result<()> {
    if sink.last_flush == sink.bytes.len() {
        return Ok(());
    }
    Err(unflushed_step_error())
}

#[cold]
#[inline(never)]
fn unflushed_step_error() -> crate::HandlerError {
    crate::HandlerError::Invariant(
        "a streaming step buffered bytes past its last flush boundary".to_string(),
    )
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

    use serde_json::Value;

    use super::{BufferSink, SessionOptions, StreamingSession};
    use crate::route_handler::Protocol;
    use crate::{BoundaryMode, FlushWriter, ResponseWriter, WebUIHandler};

    #[test]
    fn owned_streaming_session_supports_synchronized_hosts() {
        fn assert_send<T: Send>() {}
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send::<StreamingSession>();
        assert_send_sync::<Mutex<StreamingSession>>();
        assert_send_sync::<Arc<WebUIHandler>>();
    }

    #[test]
    fn buffer_sink_tracks_the_transport_flush_boundary() {
        // The invariant every owned step is checked against: bytes written
        // after the last flush would merge two semantic segments into one host
        // write.
        let mut sink = BufferSink::default();
        sink.write("checkpoint").expect("write");
        sink.flush().expect("flush");
        assert!(super::verify_flush_boundary(&sink).is_ok());
        sink.write("tail").expect("write");
        assert!(super::verify_flush_boundary(&sink).is_err());
        sink.reset();
        assert!(super::verify_flush_boundary(&sink).is_ok());
    }

    #[test]
    fn every_owned_step_ends_on_a_flush_boundary() {
        let mut parser = webui_parser::HtmlParser::new();
        parser
            .parse(
                "index.html",
                concat!(
                    "<html><head></head><body>",
                    r#"<boundary name="first"><p>1</p></boundary>"#,
                    "<hr>",
                    r#"<boundary name="second"><p>2</p></boundary>"#,
                    "<footer>tail</footer></body></html>",
                ),
            )
            .expect("parse");
        let protocol = Arc::new(Protocol::new(webui_protocol::WebUIProtocol::new(
            parser.into_fragment_records(),
        )));
        let mut session = StreamingSession::new(
            Arc::new(WebUIHandler::new()),
            protocol,
            SessionOptions::new("index.html", "/"),
        )
        .expect("session");

        let state = Value::Object(serde_json::Map::new());
        let mut step = session.start(&state).expect("start");
        let mut steps = 1usize;
        while !step.done {
            step = match step.boundary.as_ref() {
                Some(boundary) => session
                    .resume(boundary.instance_id, &state, BoundaryMode::Final)
                    .expect("resume"),
                None => session.advance().expect("advance"),
            };
            steps += 1;
        }
        // start, commit, advance, commit, advance.
        assert_eq!(steps, 5);
        assert!(session.is_done());
    }
}
