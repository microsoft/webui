// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Runtime-only Node.js native addon for the WebUI framework via napi-rs.
//!
//! Provides high-performance server-side rendering by compiling the Rust
//! WebUI handler directly into a `.node` native addon — no C ABI intermediary.
//!
//! The `Protocol` class decodes pre-compiled protobuf data from `webui build`
//! once and provides buffered and streaming render methods.
//!
//! ## Usage (from Node.js)
//!
//! ```js
//! import fs from 'node:fs';
//!
//! // Load the native addon
//! const mod = { exports: {} };
//! process.dlopen(mod, './target/release/libwebui_node.dylib');
//! const { Protocol } = mod.exports;
//!
//! // Read pre-compiled protocol (from `webui build`)
//! const protocol = new Protocol(fs.readFileSync('./dist/protocol.bin'), 'webui');
//! const state = '{"name": "WebUI"}';
//!
//! // Stream rendered fragments
//! protocol.renderStream(state, 'index.html', '/', (chunk) => process.stdout.write(chunk));
//! ```

use std::sync::Arc;

use napi::bindgen_prelude::{Buffer, Either, External, Function};
use napi::Error as NapiError;
use napi_derive::napi;
use serde_json::Value;
use webui_handler::plugin::fast_v2::FastV2HydrationPlugin;
use webui_handler::plugin::fast_v3::FastV3HydrationPlugin;
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::{
    BoundaryDescriptor, BoundaryInstanceId, BoundaryKey, BoundaryMode, HandlerError,
    Protocol as HandlerProtocol, RenderCapacityHints, RenderOptions, ResponseWriter,
    SessionOptions, StreamStep as HandlerStreamStep, StreamingSession as HandlerStreamingSession,
    WebUIHandler,
};
use webui_protocol::WebUIProtocol;

const STREAM_CHUNK_SIZE: usize = 16 * 1024;

/// Inspect protocol bytes and return a JSON representation.
#[napi]
pub fn inspect(protocol_data: Buffer) -> napi::Result<String> {
    let protocol = WebUIProtocol::from_protobuf(&protocol_data)
        .map_err(|_| NapiError::from_reason("Inspect error: protocol error"))?;
    protocol
        .to_json_pretty()
        .map_err(|e| NapiError::from_reason(format!("Inspect error: Serialization error: {e}")))
}

/// A decoded protocol and its reusable deterministic indices.
///
/// Create this once when a Node server loads `protocol.bin`, then reuse it for
/// full renders, partial navigation, component loading, and token queries. The
/// selected hydration plugin is bound once at construction.
#[napi]
pub struct Protocol {
    inner: Arc<HandlerProtocol>,
    handler: Arc<WebUIHandler>,
    output_capacity_hints: RenderCapacityHints,
}

#[napi]
impl Protocol {
    /// Decode a protocol and bind its render plugin for repeated rendering.
    #[napi(constructor)]
    pub fn new(protocol_data: Buffer, plugin: Option<String>) -> napi::Result<Self> {
        let inner = Arc::new(decode_protocol(&protocol_data)?);
        let handler = Arc::new(create_handler(plugin)?);
        Ok(Self {
            inner,
            handler,
            output_capacity_hints: RenderCapacityHints::new(),
        })
    }

    /// Render from an existing JSON string into a UTF-8 Node.js buffer.
    #[napi]
    pub fn render(
        &self,
        state_json: String,
        entry: String,
        request_path: String,
    ) -> napi::Result<Buffer> {
        let state = parse_state_json(&state_json)?;
        let options = RenderOptions::new(&entry, &request_path);
        self.render_buffer(&state, &options)
    }

    /// Parse and retain an immutable, process-local state snapshot.
    ///
    /// The returned N-API external owns the parsed state until its JavaScript
    /// handle is garbage-collected.
    #[napi]
    pub fn prepare_state(&self, state_json: String) -> napi::Result<External<Value>> {
        let size_hint = state_json.len().saturating_mul(2);
        Ok(External::new_with_size_hint(
            parse_state_json(&state_json)?,
            size_hint,
        ))
    }

    /// Render an immutable state snapshot prepared by this process.
    #[napi]
    pub fn render_prepared(
        &self,
        state: &External<Value>,
        entry: String,
        request_path: String,
    ) -> napi::Result<Buffer> {
        let options = RenderOptions::new(&entry, &request_path);
        self.render_buffer(state.as_ref(), &options)
    }

    /// Stream an existing JSON string in bounded chunks.
    #[napi]
    pub fn render_stream(
        &self,
        state_json: String,
        entry: String,
        request_path: String,
        on_chunk: Function<String>,
    ) -> napi::Result<()> {
        let state = parse_state_json(&state_json)?;
        let options = RenderOptions::new(&entry, &request_path);
        render_to_callback(&self.handler, &self.inner, &state, &options, &on_chunk)
    }

    /// Produce a complete partial-navigation response.
    #[napi]
    pub fn render_partial(
        &self,
        state_json: String,
        entry_id: String,
        request_path: String,
        inventory_hex: String,
    ) -> napi::Result<String> {
        self.inner
            .render_partial_json(&state_json, &entry_id, &request_path, &inventory_hex)
            .map_err(|e| NapiError::from_reason(format!("render_partial failed: {e}")))
    }

    /// Render component templates and styles for on-demand loading.
    #[napi]
    pub fn render_component_templates(
        &self,
        component_tags: Vec<String>,
        inventory_hex: String,
    ) -> napi::Result<String> {
        let tag_refs: Vec<&str> = component_tags.iter().map(String::as_str).collect();
        let result = self
            .inner
            .render_component_templates(&tag_refs, &inventory_hex)
            .map_err(|e| {
                NapiError::from_reason(format!("render_component_templates failed: {e}"))
            })?;
        serde_json::to_string(&result)
            .map_err(|e| NapiError::from_reason(format!("JSON serialize error: {e}")))
    }

    /// Return CSS token names in build order.
    #[napi]
    pub fn tokens(&self) -> Vec<String> {
        self.inner.tokens().to_vec()
    }

    /// Open a host-driven progressive response for a streaming entry.
    ///
    /// Unlike `renderStream`, which pushes every chunk during one synchronous
    /// call, the returned session hands each chunk back so the Node server
    /// owns the socket, the write order, and backpressure.
    #[napi]
    pub fn stream_response(
        &self,
        entry: String,
        request_path: String,
        options: Option<JsStreamOptions>,
    ) -> napi::Result<StreamingSession> {
        let options = options.unwrap_or_default();
        let mut session_options = SessionOptions::new(entry, request_path);
        session_options.nonce = options.nonce;
        session_options.head_inject = options.head_inject;
        session_options.body_inject = options.body_inject;
        let inner = HandlerStreamingSession::new(
            Arc::clone(&self.handler),
            Arc::clone(&self.inner),
            session_options,
        )
        .map_err(streaming_error)?;
        Ok(StreamingSession { inner })
    }
}

impl Protocol {
    fn render_buffer(&self, state: &Value, options: &RenderOptions<'_>) -> napi::Result<Buffer> {
        let capacity = self
            .output_capacity_hints
            .load(options.entry_id, options.request_path);
        let mut html = render_to_string(&self.handler, &self.inner, state, options, capacity)?;
        self.output_capacity_hints
            .store(options.entry_id, options.request_path, html.len());
        // Bound a stale same-route hint without reallocating normally sized output.
        if html.capacity() > html.len().saturating_mul(2) {
            html.shrink_to_fit();
        }
        // napi-rs can expose the existing Vec as an external Buffer on Node.
        Ok(Buffer::from(html.into_bytes()))
    }
}

/// Optional per-response streaming settings.
#[napi(object)]
#[derive(Default)]
pub struct JsStreamOptions {
    /// CSP nonce applied to generated inline `<script>` tags.
    pub nonce: Option<String>,
    /// HTML injected at the structural `head_end` boundary.
    pub head_inject: Option<String>,
    /// HTML injected at the structural `body_end` boundary.
    pub body_inject: Option<String>,
}

/// A progressive HTML response driven one semantic step at a time from Node.
///
/// `start()`, `resume()`, and `advance()` return bytes, completion state, and
/// the next runtime boundary occurrence (if any). Boundary keys retain their
/// authored JSON type: strings are JavaScript strings and finite numbers are
/// JavaScript numbers.
///
/// ```js
/// const session = protocol.streamResponse('index.html', '/');
/// let step = session.start(JSON.stringify(shellState));
/// res.write(step.bytes);
/// while (!step.done) {
///   const { instanceId, name, key } = step.boundary;
///   const state = await loadBoundary(name, key);
///   step = session.resume(instanceId, JSON.stringify(state), 'updatable');
///   res.write(step.bytes);
///   step = session.advance();
///   res.write(step.bytes);
/// }
/// ```
#[napi]
pub struct StreamingSession {
    inner: HandlerStreamingSession,
}

/// Runtime boundary occurrence returned by a streaming step.
#[napi(object)]
pub struct JsBoundaryDescriptor {
    /// Gapless response-local occurrence ID passed to `resume()` and `update()`.
    pub instance_id: u32,
    /// Stable compiler declaration ID.
    pub declaration_id: u32,
    /// Entry or component template that owns this declaration.
    pub owner: String,
    /// Free-form authored boundary name.
    pub name: String,
    /// Evaluated boundary key, preserving string-versus-number identity.
    pub key: Option<Either<String, f64>>,
}

/// Bytes and continuation state returned by one streaming session step.
#[napi(object)]
pub struct JsStreamStep {
    /// Complete bytes produced by this semantic step.
    pub bytes: Buffer,
    /// Whether the terminal record was emitted.
    pub done: bool,
    /// Next runtime boundary occurrence waiting for `resume()`.
    pub boundary: Option<JsBoundaryDescriptor>,
}

#[napi]
impl StreamingSession {
    /// Render until the first runtime boundary occurrence or terminal.
    #[napi]
    pub fn start(&mut self, state_json: String) -> napi::Result<JsStreamStep> {
        let state = parse_state_json(&state_json)?;
        self.inner
            .start(state)
            .and_then(stream_step)
            .map_err(streaming_error)
    }

    /// Commit the pending occurrence through its checkpoint, then stop.
    ///
    /// `mode` is `"final"` (default) or `"updatable"`. Only updatable
    /// boundaries accept later `update()` calls.
    #[napi]
    pub fn resume(
        &mut self,
        instance_id: u32,
        state_json: String,
        mode: Option<String>,
    ) -> napi::Result<JsStreamStep> {
        let state = parse_state_json(&state_json)?;
        let mode = parse_boundary_mode(mode.as_deref())?;
        self.inner
            .resume(BoundaryInstanceId::from_raw(instance_id), state, mode)
            .and_then(stream_step)
            .map_err(streaming_error)
    }

    /// Write the parent bytes after the committed occurrence.
    ///
    /// Valid only after `resume()`. Returns the next boundary occurrence or
    /// completes the document tail.
    #[napi]
    pub fn advance(&mut self) -> napi::Result<JsStreamStep> {
        self.inner
            .advance()
            .and_then(stream_step)
            .map_err(streaming_error)
    }

    /// Push a projected state patch to a committed updatable boundary.
    #[napi]
    pub fn update(&mut self, instance_id: u32, patch_json: String) -> napi::Result<Buffer> {
        let patch = parse_state_json(&patch_json)?;
        self.inner
            .update(BoundaryInstanceId::from_raw(instance_id), &patch)
            .map(Buffer::from)
            .map_err(streaming_error)
    }
}

fn stream_step(step: HandlerStreamStep) -> webui_handler::Result<JsStreamStep> {
    Ok(JsStreamStep {
        bytes: Buffer::from(step.bytes),
        done: step.done,
        boundary: step.boundary.map(boundary_descriptor).transpose()?,
    })
}

fn boundary_descriptor(
    boundary: BoundaryDescriptor,
) -> webui_handler::Result<JsBoundaryDescriptor> {
    Ok(JsBoundaryDescriptor {
        instance_id: boundary.instance_id.raw(),
        declaration_id: boundary.declaration_id,
        owner: boundary.owner.to_string(),
        name: boundary.name.to_string(),
        key: boundary.key.map(boundary_key).transpose()?,
    })
}

fn boundary_key(key: BoundaryKey) -> webui_handler::Result<Either<String, f64>> {
    match key {
        BoundaryKey::String(value) => Ok(Either::A(value)),
        BoundaryKey::Number(value) => value.as_f64().map(Either::B).ok_or_else(|| {
            HandlerError::Invariant(
                "boundary key cannot be represented as a JavaScript number".into(),
            )
        }),
    }
}

fn parse_boundary_mode(mode: Option<&str>) -> napi::Result<BoundaryMode> {
    match mode {
        None | Some("final") => Ok(BoundaryMode::Final),
        Some("updatable") => Ok(BoundaryMode::Updatable),
        Some(other) => Err(NapiError::from_reason(format!(
            "unknown boundary mode '{other}'; expected 'final' or 'updatable'"
        ))),
    }
}

fn streaming_error(error: HandlerError) -> NapiError {
    NapiError::from_reason(error.to_string())
}

fn decode_protocol(protocol_data: &[u8]) -> napi::Result<HandlerProtocol> {
    HandlerProtocol::from_protobuf(protocol_data)
        .map_err(|e| NapiError::from_reason(format!("Protocol decode error: {e}")))
}

fn parse_state_json(state_json: &str) -> napi::Result<Value> {
    serde_json::from_str(state_json)
        .map_err(|e| NapiError::from_reason(format!("State JSON error: {e}")))
}

fn create_handler(plugin: Option<String>) -> napi::Result<WebUIHandler> {
    // The handler exposes plugin factories, not the compiler's Plugin enum.
    // Keep this compatibility adapter aligned with the compiler via dev-only tests.
    Ok(match plugin.as_deref() {
        Some("fast" | "fast-v2") => {
            WebUIHandler::with_plugin(|| Box::new(FastV2HydrationPlugin::new()))
        }
        Some("fast-v3") => WebUIHandler::with_plugin(|| Box::new(FastV3HydrationPlugin::new())),
        Some("webui") => WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new())),
        None => WebUIHandler::new(),
        Some(other) => return Err(unknown_plugin(other)),
    })
}

#[cold]
#[inline(never)]
fn unknown_plugin(plugin: &str) -> NapiError {
    NapiError::from_reason(format!(
        "Unknown plugin: {plugin}. Use \"webui\", \"fast-v3\", \"fast-v2\", or \"fast\"."
    ))
}

webui_handler::define_string_response_writer!(BufferedWriter, output);

fn render_to_string(
    handler: &WebUIHandler,
    protocol: &HandlerProtocol,
    state: &Value,
    options: &RenderOptions<'_>,
    capacity: usize,
) -> napi::Result<String> {
    let mut writer = BufferedWriter::with_capacity(capacity);
    handler
        .render(protocol, state, options, &mut writer)
        .map_err(|e| NapiError::from_reason(format!("Render error: {e}")))?;
    Ok(writer.output)
}

/// A writer that batches rendered fragments before crossing into JavaScript.
struct CallbackWriter<F> {
    callback: F,
    buffer: String,
    error: Option<NapiError>,
}

impl<F> CallbackWriter<F>
where
    F: FnMut(String) -> napi::Result<()>,
{
    fn new(callback: F) -> Self {
        Self {
            callback,
            buffer: String::with_capacity(STREAM_CHUNK_SIZE),
            error: None,
        }
    }

    fn flush(&mut self) -> webui_handler::Result<()> {
        if self.error.is_some() {
            return Err(callback_writer_error());
        }
        if self.buffer.is_empty() {
            return Ok(());
        }

        let chunk = std::mem::replace(&mut self.buffer, String::with_capacity(STREAM_CHUNK_SIZE));
        if let Err(error) = (self.callback)(chunk) {
            self.error = Some(error);
            return Err(callback_writer_error());
        }
        Ok(())
    }
}

impl<F> ResponseWriter for CallbackWriter<F>
where
    F: FnMut(String) -> napi::Result<()>,
{
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        if self.error.is_some() {
            return Err(callback_writer_error());
        }
        self.buffer.push_str(content);
        if self.buffer.len() >= STREAM_CHUNK_SIZE {
            self.flush()?;
        }
        Ok(())
    }

    fn write_attribute(&mut self, name: &str, value: &str) -> webui_handler::Result<()> {
        if self.error.is_some() {
            return Err(callback_writer_error());
        }
        webui_handler::append_attribute_to_string(&mut self.buffer, name, value);
        if self.buffer.len() >= STREAM_CHUNK_SIZE {
            self.flush()?;
        }
        Ok(())
    }

    fn write_boolean_attribute(&mut self, name: &str) -> webui_handler::Result<()> {
        if self.error.is_some() {
            return Err(callback_writer_error());
        }
        webui_handler::append_boolean_attribute_to_string(&mut self.buffer, name);
        if self.buffer.len() >= STREAM_CHUNK_SIZE {
            self.flush()?;
        }
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        self.flush()
    }
}

#[cold]
#[inline(never)]
fn callback_writer_error() -> HandlerError {
    HandlerError::Writer("Node chunk callback failed".to_owned())
}

fn render_to_callback(
    handler: &WebUIHandler,
    protocol: &HandlerProtocol,
    state: &Value,
    options: &RenderOptions<'_>,
    on_chunk: &Function<String>,
) -> napi::Result<()> {
    let mut writer = CallbackWriter::new(|chunk| on_chunk.call(chunk).map(drop));
    let render_result = handler.render(protocol, state, options, &mut writer);
    if let Some(error) = writer.error {
        return Err(error);
    }
    render_result.map_err(|e| NapiError::from_reason(format!("Render error: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use webui_parser::HtmlParser;

    #[test]
    fn handler_plugin_names_and_errors_match_compiler_contract() -> napi::Result<()> {
        for plugin in ["fast", "fast-v2", "fast-v3", "webui"] {
            assert!(plugin.parse::<webui_parser::Plugin>().is_ok());
            create_handler(Some(plugin.to_string()))?;
        }
        create_handler(None)?;

        for plugin in ["", "none", "FAST", "webui ", "unknown"] {
            let compiler_error = plugin
                .parse::<webui_parser::Plugin>()
                .err()
                .ok_or_else(|| NapiError::from_reason("compiler accepted invalid plugin"))?;
            let runtime_error = create_handler(Some(plugin.to_string()))
                .err()
                .ok_or_else(|| NapiError::from_reason("runtime accepted invalid plugin"))?;
            assert_eq!(runtime_error.status, napi::Status::GenericFailure);
            assert_eq!(runtime_error.reason, compiler_error);
        }
        Ok(())
    }

    #[test]
    fn fast_alias_preserves_fast_v2_rendering() -> napi::Result<()> {
        let bytes = build_protocol("<p>{{name}}</p>");
        let render = |plugin: &str| {
            Protocol::new(Buffer::from(bytes.clone()), Some(plugin.to_string()))?.render(
                r#"{"name":"WebUI"}"#.to_string(),
                "index.html".to_string(),
                "/".to_string(),
            )
        };
        assert_eq!(render("fast")?.as_ref(), render("fast-v2")?.as_ref());
        Ok(())
    }

    /// Helper: parse HTML into protobuf bytes for testing.
    fn build_protocol(html: &str) -> Vec<u8> {
        let mut parser = HtmlParser::new();
        parser.parse("index.html", html).expect("parse failed");
        let tokens = parser.take_tokens();
        let protocol = WebUIProtocol::with_tokens(parser.into_fragment_records(), tokens);
        protocol.to_protobuf().expect("protobuf encode failed")
    }

    /// Helper: render protocol bytes + state, collecting output into a String.
    fn render_to_string(protocol_bytes: &[u8], state_json: &str) -> Result<String, String> {
        let protocol = HandlerProtocol::from_protobuf(protocol_bytes).map_err(|e| e.to_string())?;
        let state: Value = serde_json::from_str(state_json).map_err(|e| e.to_string())?;

        let mut output = String::with_capacity(1024);
        let handler = WebUIHandler::new();

        struct StringWriter<'a> {
            output: &'a mut String,
        }
        impl ResponseWriter for StringWriter<'_> {
            fn write(&mut self, content: &str) -> webui_handler::Result<()> {
                self.output.push_str(content);
                Ok(())
            }
            fn end(&mut self) -> webui_handler::Result<()> {
                Ok(())
            }
        }

        let mut writer = StringWriter {
            output: &mut output,
        };
        handler
            .render(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .map_err(|e| e.to_string())?;
        Ok(output)
    }

    #[test]
    fn learned_render_capacity_avoids_string_growth_without_changing_bytes() {
        struct ReallocationWriter {
            output: String,
            reallocations: usize,
        }

        impl ResponseWriter for ReallocationWriter {
            fn write(&mut self, content: &str) -> webui_handler::Result<()> {
                let previous_capacity = self.output.capacity();
                self.output.push_str(content);
                self.reallocations += usize::from(self.output.capacity() != previous_capacity);
                Ok(())
            }

            fn end(&mut self) -> webui_handler::Result<()> {
                Ok(())
            }
        }

        fn render_with_capacity(protocol: &HandlerProtocol, capacity: usize) -> (String, usize) {
            let mut writer = ReallocationWriter {
                output: String::with_capacity(capacity),
                reallocations: 0,
            };
            WebUIHandler::new()
                .render(
                    protocol,
                    &Value::Null,
                    &RenderOptions::new("index.html", "/"),
                    &mut writer,
                )
                .expect("render should succeed");
            (writer.output, writer.reallocations)
        }

        let source = format!("<main>{}</main>", "<p>capacity</p>".repeat(1024));
        let protocol =
            HandlerProtocol::from_protobuf(&build_protocol(&source)).expect("protocol should load");
        let (cold_output, cold_reallocations) = render_with_capacity(
            &protocol,
            RenderCapacityHints::new().load("index.html", "/"),
        );
        let (warm_output, warm_reallocations) = render_with_capacity(&protocol, cold_output.len());

        assert!(cold_reallocations > 0);
        assert_eq!(warm_reallocations, 0);
        assert_eq!(warm_output, cold_output);
    }

    #[test]
    fn test_simple_passthrough() {
        let proto = build_protocol("<p>Hello</p>");
        let result = render_to_string(&proto, "{}");
        assert_eq!(result.as_deref(), Ok("<p>Hello</p>"));
    }

    #[test]
    fn test_signal_substitution() {
        let proto = build_protocol("Hello, {{name}}!");
        let result = render_to_string(&proto, r#"{"name": "WebUI"}"#);
        assert_eq!(result.as_deref(), Ok("Hello, WebUI!"));
    }

    #[test]
    fn protocol_reuses_decoded_protocol_for_json_state() {
        let proto = build_protocol("Hello, {{name}}!");
        let protocol = Protocol::new(Buffer::from(proto), None).expect("protocol should load");

        let first = protocol
            .render(
                r#"{"name":"First"}"#.to_string(),
                "index.html".to_string(),
                "/".to_string(),
            )
            .expect("first render should succeed");
        let second = protocol
            .render(
                r#"{"name":"世界"}"#.to_string(),
                "index.html".to_string(),
                "/".to_string(),
            )
            .expect("second render should succeed");

        assert_eq!(first.as_ref(), b"Hello, First!");
        assert_eq!(second.as_ref(), "Hello, 世界!".as_bytes());
    }

    #[test]
    fn streaming_steps_preserve_key_types_and_checkpoint_segments() {
        let proto = build_protocol(concat!(
            "<html><head></head><body>",
            r#"<boundary name="first" key="{{firstId}}"><p>{{firstLabel}}</p></boundary>"#,
            "<span>between</span>",
            r#"<boundary name="second" key="{{secondId}}"><p>{{secondLabel}}</p></boundary>"#,
            "<footer>tail</footer>",
            "</body></html>",
        ));
        let protocol = Protocol::new(Buffer::from(proto), None).expect("protocol should load");
        let mut session = protocol
            .stream_response("index.html".to_string(), "/".to_string(), None)
            .expect("session should open");
        let state = r#"{"firstId":"alpha","firstLabel":"a","secondId":20,"secondLabel":"b"}"#;

        let first = session
            .start(state.to_string())
            .expect("start should discover first boundary");
        assert!(!first.done);
        assert!(!first.bytes.is_empty());
        let first_boundary = first.boundary.expect("first boundary should be returned");
        assert_eq!(first_boundary.instance_id, 0);
        assert_eq!(first_boundary.declaration_id, 0);
        assert_eq!(first_boundary.owner, "index.html");
        assert_eq!(first_boundary.name, "first");
        assert!(matches!(
            first_boundary.key,
            Some(Either::A(value)) if value == "alpha"
        ));

        let resumed = session
            .resume(
                first_boundary.instance_id,
                state.to_string(),
                Some("final".to_string()),
            )
            .expect("resume should commit first boundary");
        assert!(!resumed.done);
        assert!(resumed.boundary.is_none());
        let resumed_text =
            std::str::from_utf8(&resumed.bytes).expect("resume output should be UTF-8");
        assert!(resumed_text.contains(">a<"));
        assert!(!resumed_text.contains("between"));

        let next = session
            .advance()
            .expect("advance should discover second boundary");
        assert!(!next.done);
        let next_text = std::str::from_utf8(&next.bytes).expect("advance output should be UTF-8");
        assert!(next_text.contains("between"));
        assert!(!next_text.contains(">b<"));
        let second_boundary = next.boundary.expect("second boundary should be returned");
        assert_eq!(second_boundary.instance_id, 1);
        assert_eq!(second_boundary.declaration_id, 1);
        assert_eq!(second_boundary.name, "second");
        assert!(matches!(
            second_boundary.key,
            Some(Either::B(value)) if value == 20.0
        ));

        let resumed = session
            .resume(second_boundary.instance_id, state.to_string(), None)
            .expect("resume should commit second boundary");
        assert!(!resumed.done);
        assert!(resumed.boundary.is_none());
        let resumed_text =
            std::str::from_utf8(&resumed.bytes).expect("resume output should be UTF-8");
        assert!(resumed_text.contains(">b<"));
        assert!(!resumed_text.contains("tail"));

        let done = session.advance().expect("final advance should complete");
        assert!(done.done);
        assert!(done.boundary.is_none());
        assert!(std::str::from_utf8(&done.bytes)
            .expect("advance output should be UTF-8")
            .contains("tail"));
    }

    #[test]
    fn streaming_update_returns_buffer_for_updatable_occurrence() {
        let proto = build_protocol(concat!(
            "<html><head></head><body>",
            r#"<boundary name="first"><p>{{count}}</p></boundary>"#,
            r#"<boundary name="second"><p>done</p></boundary>"#,
            "</body></html>",
        ));
        let protocol = Protocol::new(Buffer::from(proto), None).expect("protocol should load");
        let mut session = protocol
            .stream_response("index.html".to_string(), "/".to_string(), None)
            .expect("session should open");
        let first = session
            .start(r#"{"count":1}"#.to_string())
            .expect("start should discover first boundary")
            .boundary
            .expect("first boundary should be returned");
        let resumed = session
            .resume(
                first.instance_id,
                r#"{"count":1}"#.to_string(),
                Some("updatable".to_string()),
            )
            .expect("resume should commit updatable boundary");
        assert!(!resumed.done);
        assert!(resumed.boundary.is_none());

        let update = session
            .update(first.instance_id, r#"{"count":2}"#.to_string())
            .expect("update should render");
        assert!(!update.is_empty());
        let update_text = std::str::from_utf8(&update).expect("update should be UTF-8");
        assert!(update_text.contains(r#""count":2"#));

        let second = session
            .advance()
            .expect("advance should discover second boundary")
            .boundary
            .expect("second boundary should be returned");
        let resumed = session
            .resume(second.instance_id, "{}".to_string(), None)
            .expect("second resume should commit boundary");
        assert!(!resumed.done);
        assert!(resumed.boundary.is_none());
        let done = session.advance().expect("final advance should complete");
        assert!(done.done);
    }

    #[test]
    fn streaming_advance_rejects_out_of_order_calls() {
        let proto = build_protocol(concat!(
            "<html><head></head><body>",
            r#"<boundary name="first"><p>first</p></boundary>"#,
            "</body></html>",
        ));
        let protocol = Protocol::new(Buffer::from(proto), None).expect("protocol should load");
        let mut session = protocol
            .stream_response("index.html".to_string(), "/".to_string(), None)
            .expect("session should open");

        let before_start = session
            .advance()
            .err()
            .expect("advance before start should fail");
        assert!(before_start
            .to_string()
            .contains("start must be called before this operation"));

        let start = session
            .start("{}".to_string())
            .expect("start should succeed");
        let before_resume = session
            .advance()
            .err()
            .expect("advance before resume should fail");
        assert!(before_resume
            .to_string()
            .contains("there is no committed boundary to advance past"));

        let boundary = start.boundary.expect("first boundary should be returned");
        session
            .resume(boundary.instance_id, "{}".to_string(), None)
            .expect("resume should still succeed after rejected advance");
        assert!(session.advance().expect("advance should complete").done);
    }

    #[test]
    fn streaming_start_returns_done_for_boundary_free_document() {
        let proto = build_protocol("<html><head></head><body><p>done</p></body></html>");
        let protocol = Protocol::new(Buffer::from(proto), None).expect("protocol should load");
        let mut session = protocol
            .stream_response("index.html".to_string(), "/".to_string(), None)
            .expect("session should open");

        let step = session
            .start("{}".to_string())
            .expect("boundary-free start should complete");
        assert!(step.done);
        assert!(step.boundary.is_none());
        assert!(std::str::from_utf8(&step.bytes)
            .expect("output should be UTF-8")
            .contains("<p>done</p>"));
    }

    #[test]
    fn test_for_loop() {
        let proto = build_protocol("<ul><for each=\"item in items\"><li>{{item}}</li></for></ul>");
        let result = render_to_string(&proto, r#"{"items": ["a", "b", "c"]}"#);
        assert_eq!(
            result.as_deref(),
            Ok("<ul><li>a</li><li>b</li><li>c</li></ul>")
        );
    }

    #[test]
    fn test_if_condition_true() {
        let proto = build_protocol("<if condition=\"show\"><p>Visible</p></if>");
        let result = render_to_string(&proto, r#"{"show": true}"#);
        assert_eq!(result.as_deref(), Ok("<p>Visible</p>"));
    }

    #[test]
    fn test_if_condition_false() {
        let proto = build_protocol("<if condition=\"show\"><p>Hidden</p></if>");
        let result = render_to_string(&proto, r#"{"show": false}"#);
        assert_eq!(result.as_deref(), Ok(""));
    }

    #[test]
    fn test_html_escaping() {
        let proto = build_protocol("<div>{{content}}</div>");
        let state = r#"{"content": "<script>alert('xss')</script>"}"#;
        let result = render_to_string(&proto, state).expect("render should succeed");
        assert!(!result.contains("<script>"));
        assert!(result.contains("&lt;script&gt;"));
    }

    #[test]
    fn test_raw_signal() {
        let proto = build_protocol("<div>{{{content}}}</div>");
        let result = render_to_string(&proto, r#"{"content": "<b>bold</b>"}"#);
        assert_eq!(result.as_deref(), Ok("<div><b>bold</b></div>"));
    }

    #[test]
    fn test_invalid_json() {
        let proto = build_protocol("<p>hi</p>");
        let result = render_to_string(&proto, "NOT JSON");
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_state() {
        let proto = build_protocol("<p>static</p>");
        let result = render_to_string(&proto, "{}");
        assert_eq!(result.as_deref(), Ok("<p>static</p>"));
    }

    #[test]
    fn test_nested_object_signal() {
        let proto = build_protocol("{{user.name}}");
        let result = render_to_string(&proto, r#"{"user": {"name": "Alice"}}"#);
        assert_eq!(result.as_deref(), Ok("Alice"));
    }

    #[test]
    fn test_invalid_protobuf() {
        let result = render_to_string(&[0xFF, 0xFF, 0xFF], "{}");
        assert!(result.is_err());
    }

    /// Parse `html`, attach a sorted hydration `schema`, and encode to protobuf.
    fn build_projected_protocol(html: &str, schema: &[&str]) -> Vec<u8> {
        let mut parser = HtmlParser::new();
        parser.parse("index.html", html).expect("parse failed");
        let tokens = parser.take_tokens();
        let mut protocol = WebUIProtocol::with_tokens(parser.into_fragment_records(), tokens);
        protocol.fragments.insert(
            "client-card".to_string(),
            webui_protocol::FragmentList {
                fragments: vec![webui_protocol::WebUIFragment::raw("<p>client</p>")],
                contains_boundary: false,
            },
        );
        protocol
            .fragments
            .get_mut("index.html")
            .expect("index fragment should exist")
            .fragments
            .insert(1, webui_protocol::WebUIFragment::component("client-card"));
        protocol.initial_state_strategy = webui_protocol::InitialStateStrategy::Components as i32;
        protocol.components.insert(
            "client-card".to_string(),
            webui_protocol::ComponentData {
                hydration_mode: webui_protocol::StateProjectionMode::Keys as i32,
                hydration_keys: schema.iter().map(|key| (*key).to_string()).collect(),
                ..Default::default()
            },
        );
        protocol.to_protobuf().expect("protobuf encode failed")
    }

    /// Render protocol bytes with the WebUI hydration plugin so the `#webui-data`
    /// bootstrap block (and its projected state) is emitted — this mirrors the
    /// production `render(..., plugin = "webui")` path.
    fn render_with_webui_plugin(protocol_bytes: &[u8], state_json: &str) -> Result<String, String> {
        let protocol = HandlerProtocol::from_protobuf(protocol_bytes).map_err(|e| e.to_string())?;
        let state: Value = serde_json::from_str(state_json).map_err(|e| e.to_string())?;

        let mut output = String::with_capacity(1024);
        struct StringWriter<'a> {
            output: &'a mut String,
        }
        impl ResponseWriter for StringWriter<'_> {
            fn write(&mut self, content: &str) -> webui_handler::Result<()> {
                self.output.push_str(content);
                Ok(())
            }
            fn end(&mut self) -> webui_handler::Result<()> {
                Ok(())
            }
        }
        let mut writer = StringWriter {
            output: &mut output,
        };
        let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
        handler
            .render(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .map_err(|e| e.to_string())?;
        Ok(output)
    }

    #[test]
    fn render_projects_state_to_component_hydration_keys() {
        // Full document so the parser emits a `body_end` signal, which makes the
        // WebUI plugin emit the #webui-data bootstrap block.
        let bytes =
            build_projected_protocol("<html><body><p>{{kept}}</p></body></html>", &["kept"]);
        let out =
            render_with_webui_plugin(&bytes, r#"{"kept":"KEPT_VALUE","dropped":"DROPPED_VALUE"}"#)
                .expect("render should succeed");

        // Only the hydratable key reaches the bootstrap state block...
        assert!(
            out.contains(r#""kept":"KEPT_VALUE""#),
            "hydratable key missing from bootstrap state: {out}"
        );
        // ...the non-hydratable key is projected out entirely.
        assert!(
            !out.contains("DROPPED_VALUE"),
            "server-only value leaked: {out}"
        );
        assert!(
            !out.contains("dropped"),
            "server-only key name leaked: {out}"
        );
    }

    #[test]
    fn test_inspect_valid_protocol() {
        let proto = build_protocol("<h1>Hello {{name}}</h1>");
        let json = inspect(napi::bindgen_prelude::Buffer::from(proto)).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("fragments").is_some());
    }

    #[test]
    fn test_inspect_invalid_protocol() {
        let result = inspect(napi::bindgen_prelude::Buffer::from(vec![0xFF, 0xFF]));
        assert!(result.is_err());
    }

    #[test]
    fn inspect_preserves_invalid_protocol_error_contract() -> napi::Result<()> {
        let error = inspect(Buffer::from(vec![0xFF, 0xFF]))
            .err()
            .ok_or_else(|| NapiError::from_reason("invalid protocol was accepted"))?;
        assert_eq!(error.status, napi::Status::GenericFailure);
        assert_eq!(error.reason, "Inspect error: protocol error");
        Ok(())
    }

    #[test]
    fn test_protocol_tokens_empty() {
        let proto = build_protocol("<p>Hello</p>");
        let protocol = Protocol::new(Buffer::from(proto), None).unwrap();
        let tokens = protocol.tokens();
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_protocol_tokens_returns_parsed_tokens() {
        // Build from a protocol that has CSS tokens via with_tokens constructor.
        let mut parser = HtmlParser::new();
        parser.parse("index.html", "<p>Hi</p>").expect("parse");
        let protocol = WebUIProtocol::with_tokens(
            parser.into_fragment_records(),
            vec![
                "colorBrandBackground".to_string(),
                "fontSizeBase300".to_string(),
            ],
        );
        let proto = protocol.to_protobuf().expect("encode");
        let protocol = Protocol::new(Buffer::from(proto), None).unwrap();
        let tokens = protocol.tokens();
        assert_eq!(tokens, vec!["colorBrandBackground", "fontSizeBase300"]);
    }

    #[test]
    fn test_protocol_tokens_invalid_protobuf() {
        let result = Protocol::new(Buffer::from(vec![0xFF, 0xFF]), None);
        assert!(result.is_err());
    }
}
