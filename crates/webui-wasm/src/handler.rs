// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Handler-only WASM exports.

use crate::error::WasmError;
use js_sys::{Function, Object, Reflect};
use serde_json::Value;
use std::sync::Arc;
use wasm_bindgen::prelude::*;
use webui_handler::plugin::fast_v2::FastV2HydrationPlugin;
use webui_handler::plugin::fast_v3::FastV3HydrationPlugin;
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::{
    BoundaryId, BoundaryMode, HandlerError, Protocol as HandlerProtocol, RenderOptions,
    ResponseWriter, SessionOptions, StreamingSession as HandlerStreamingSession, WebUIHandler,
};
#[cfg(test)]
use webui_protocol::WebUIProtocol;

const STREAM_CHUNK_SIZE: usize = 16 * 1024;

/// A string buffer for collecting rendered output.
struct StringWriter {
    content: String,
}

impl StringWriter {
    fn with_capacity(cap: usize) -> Self {
        Self {
            content: String::with_capacity(cap),
        }
    }
}

impl ResponseWriter for StringWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.content.push_str(content);
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

/// A writer that batches rendered fragments before crossing into JavaScript.
struct CallbackWriter<'a> {
    on_chunk: &'a Function,
    buffer: String,
}

impl<'a> CallbackWriter<'a> {
    fn new(on_chunk: &'a Function) -> Self {
        Self {
            on_chunk,
            buffer: String::with_capacity(STREAM_CHUNK_SIZE),
        }
    }

    fn flush(&mut self) -> webui_handler::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        let chunk = std::mem::replace(&mut self.buffer, String::with_capacity(STREAM_CHUNK_SIZE));
        self.on_chunk
            .call1(&JsValue::UNDEFINED, &JsValue::from_str(&chunk))
            .map(|_| ())
            .map_err(|error| HandlerError::Writer(format!("{error:?}")))
    }
}

impl ResponseWriter for CallbackWriter<'_> {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.buffer.push_str(content);
        if self.buffer.len() >= STREAM_CHUNK_SIZE {
            self.flush()?;
        }
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        self.flush()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HandlerPluginKind {
    FastV2,
    FastV3,
    WebUI,
}

impl HandlerPluginKind {
    fn parse(name: &str) -> Result<Self, WasmError> {
        match name {
            "fast" | "fast-v2" => Ok(Self::FastV2),
            "fast-v3" => Ok(Self::FastV3),
            "webui" => Ok(Self::WebUI),
            other => Err(WasmError::UnknownPlugin(other.to_string())),
        }
    }
}

struct WasmRenderOptions {
    entry: String,
    request_path: String,
}

impl Default for WasmRenderOptions {
    fn default() -> Self {
        Self {
            entry: "index.html".to_string(),
            request_path: "/".to_string(),
        }
    }
}

/// A decoded protocol with reusable indices for repeated WASM renders.
#[wasm_bindgen]
pub struct Protocol {
    inner: Arc<HandlerProtocol>,
    handler: Arc<WebUIHandler>,
}

#[wasm_bindgen]
impl Protocol {
    /// Decode protobuf bytes once for repeated rendering.
    #[wasm_bindgen(constructor)]
    pub fn new(protocol_bytes: &[u8], plugin: Option<String>) -> Result<Protocol, JsValue> {
        let plugin = parse_optional_plugin(plugin.as_deref())
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        let inner = HandlerProtocol::from_protobuf(protocol_bytes)
            .map_err(|error| JsValue::from_str(&format!("Protocol error: {error}")))?;
        Ok(Self {
            inner: Arc::new(inner),
            handler: Arc::new(create_handler(plugin)),
        })
    }

    /// Render from an existing JSON string.
    #[wasm_bindgen(js_name = render)]
    pub fn render(&self, state_json: &str, options: Option<Object>) -> Result<String, JsValue> {
        let options =
            parse_render_options(options).map_err(|error| JsValue::from_str(&error.to_string()))?;
        let state =
            parse_state_json(state_json).map_err(|error| JsValue::from_str(&error.to_string()))?;
        render_protocol_to_string_value(&self.handler, &self.inner, &state, &options)
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// Stream from an existing JSON string in bounded chunks.
    #[wasm_bindgen(js_name = renderStream)]
    pub fn render_stream(
        &self,
        state_json: &str,
        on_chunk: &Function,
        options: Option<Object>,
    ) -> Result<(), JsValue> {
        let options =
            parse_render_options(options).map_err(|error| JsValue::from_str(&error.to_string()))?;
        let state =
            parse_state_json(state_json).map_err(|error| JsValue::from_str(&error.to_string()))?;
        render_protocol_to_callback_value(&self.handler, &self.inner, &state, &options, on_chunk)
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// Produce a complete partial-navigation response.
    #[wasm_bindgen(js_name = renderPartial)]
    pub fn render_partial(
        &self,
        state_json: &str,
        entry_id: &str,
        request_path: &str,
        inventory_hex: &str,
    ) -> Result<String, JsValue> {
        self.inner
            .render_partial(state_json, entry_id, request_path, inventory_hex)
            .map_err(|error| JsValue::from_str(&format!("render_partial failed: {error}")))
    }

    /// Return component template payloads for requested component tags.
    #[wasm_bindgen(js_name = renderComponentTemplates)]
    pub fn render_component_templates(
        &self,
        component_tags: JsValue,
        inventory_hex: &str,
    ) -> Result<String, JsValue> {
        let tags: Vec<String> = serde_wasm_bindgen::from_value(component_tags)
            .map_err(|error| JsValue::from_str(&format!("invalid component tags: {error}")))?;
        let tag_refs: Vec<&str> = tags.iter().map(String::as_str).collect();
        let result = self
            .inner
            .render_component_templates(&tag_refs, inventory_hex)
            .map_err(|error| {
                JsValue::from_str(&format!("render_component_templates failed: {error}"))
            })?;
        serde_json::to_string(&result)
            .map_err(|error| JsValue::from_str(&format!("JSON serialize error: {error}")))
    }

    /// Return CSS token names in build order.
    #[wasm_bindgen(js_name = tokens)]
    pub fn tokens(&self) -> Result<JsValue, JsValue> {
        serde_wasm_bindgen::to_value(self.inner.tokens())
            .map_err(|error| JsValue::from_str(&format!("Serialization error: {error}")))
    }

    /// Open a host-driven progressive response for a streaming entry.
    ///
    /// Unlike `renderStream`, which pushes every chunk through one callback
    /// during a single synchronous call, the returned session hands each chunk
    /// back so the host owns the socket, the write order, and backpressure.
    #[wasm_bindgen(js_name = streamResponse)]
    pub fn stream_response(
        &self,
        entry: String,
        request_path: String,
        options: Option<Object>,
    ) -> Result<StreamingSession, JsValue> {
        let mut session_options = SessionOptions::new(entry, request_path);
        if let Some(options) = options {
            session_options.nonce = optional_string_property(&options, "nonce")?;
            session_options.head_inject = optional_string_property(&options, "headInject")?;
            session_options.body_inject = optional_string_property(&options, "bodyInject")?;
        }

        HandlerStreamingSession::new(
            Arc::clone(&self.handler),
            Arc::clone(&self.inner),
            session_options,
        )
        .map(|inner| StreamingSession { inner })
        .map_err(streaming_error)
    }
}

/// A progressive HTML response driven one chunk at a time from JavaScript.
///
/// Every method returns the UTF-8 bytes it produced. Write them to the
/// response and apply the host's own backpressure; the session holds no
/// transport and never blocks on one.
///
/// ```js
/// const session = protocol.streamResponse('index.html', '/');
/// const weather = session.boundary('weather-shell');
/// controller.enqueue(session.writeShell(shellState));
/// controller.enqueue(session.writeBoundary(weather, weatherState, 'updatable'));
/// controller.enqueue(session.update(weather, forecast));
/// controller.enqueue(session.finish(tailState));
/// ```
#[wasm_bindgen]
pub struct StreamingSession {
    inner: HandlerStreamingSession,
}

#[wasm_bindgen]
impl StreamingSession {
    /// Resolve an authored boundary name to a stable integer handle.
    ///
    /// Resolve once outside the write loop; the handle costs nothing to reuse.
    #[wasm_bindgen(js_name = boundary)]
    pub fn boundary(&self, name: &str) -> Result<u32, JsValue> {
        self.inner
            .boundary(name)
            .map(BoundaryId::raw)
            .map_err(streaming_error)
    }

    /// Number of compile-time boundaries declared by this entry.
    #[wasm_bindgen(getter, js_name = boundaryCount)]
    pub fn boundary_count(&self) -> u32 {
        // Boundary counts are bounded by the compiled entry, so this cannot
        // exceed u32 in any protocol the build can produce.
        u32::try_from(self.inner.boundary_count()).unwrap_or(u32::MAX)
    }

    /// Whether the terminal record has been written.
    #[wasm_bindgen(getter, js_name = finished)]
    pub fn finished(&self) -> bool {
        self.inner.is_finished()
    }

    /// Render everything before the first boundary.
    #[wasm_bindgen(js_name = writeShell)]
    pub fn write_shell(&mut self, state_json: &str) -> Result<Vec<u8>, JsValue> {
        let state = session_state(state_json)?;

        self.inner.write_shell(&state).map_err(streaming_error)
    }

    /// Render and commit the next boundary in declaration order.
    ///
    /// `mode` is `"final"` (default) or `"updatable"`. Only updatable
    /// boundaries accept later `update()` calls.
    #[wasm_bindgen(js_name = writeBoundary)]
    pub fn write_boundary(
        &mut self,
        boundary: u32,
        state_json: &str,
        mode: Option<String>,
    ) -> Result<Vec<u8>, JsValue> {
        let state = session_state(state_json)?;

        let mode = parse_boundary_mode(mode.as_deref())?;
        self.inner
            .write_boundary(BoundaryId::from_raw(boundary), &state, mode)
            .map_err(streaming_error)
    }

    /// Push a projected state patch to a committed updatable boundary.
    #[wasm_bindgen(js_name = update)]
    pub fn update(&mut self, boundary: u32, state_json: &str) -> Result<Vec<u8>, JsValue> {
        let state = session_state(state_json)?;

        self.inner
            .update(BoundaryId::from_raw(boundary), &state)
            .map_err(streaming_error)
    }

    /// Render the document tail and emit the terminal record.
    #[wasm_bindgen(js_name = finish)]
    pub fn finish(&mut self, state_json: &str) -> Result<Vec<u8>, JsValue> {
        let state = session_state(state_json)?;

        self.inner.finish(&state).map_err(streaming_error)
    }
}

fn session_state(state_json: &str) -> Result<Value, JsValue> {
    parse_state_json(state_json).map_err(|error| JsValue::from_str(&error.to_string()))
}

fn parse_boundary_mode(mode: Option<&str>) -> Result<BoundaryMode, JsValue> {
    match mode {
        None | Some("final") => Ok(BoundaryMode::Final),
        Some("updatable") => Ok(BoundaryMode::Updatable),
        Some(other) => Err(JsValue::from_str(&format!(
            "unknown boundary mode '{other}'; expected 'final' or 'updatable'"
        ))),
    }
}

fn streaming_error(error: HandlerError) -> JsValue {
    JsValue::from_str(&error.to_string())
}

fn optional_string_property(options: &Object, key: &str) -> Result<Option<String>, JsValue> {
    let value = Reflect::get(options, &JsValue::from_str(key))
        .map_err(|_| JsValue::from_str(&format!("failed to read '{key}' option")))?;
    if value.is_undefined() || value.is_null() {
        return Ok(None);
    }
    value
        .as_string()
        .map(Some)
        .ok_or_else(|| JsValue::from_str(&format!("'{key}' must be a string")))
}

#[cfg(test)]
pub(crate) fn render_protocol_to_string(
    protocol: &WebUIProtocol,
    state_json: &str,
    entry: &str,
    request_path: &str,
    plugin: Option<HandlerPluginKind>,
) -> Result<String, WasmError> {
    let state = parse_state_json(state_json)?;
    let options = WasmRenderOptions {
        entry: entry.to_string(),
        request_path: request_path.to_string(),
    };
    let protocol = HandlerProtocol::new(protocol.clone());
    let handler = create_handler(plugin);
    render_protocol_to_string_value(&handler, &protocol, &state, &options)
}

fn parse_state_json(state_json: &str) -> Result<Value, WasmError> {
    serde_json::from_str(state_json).map_err(WasmError::State)
}

fn render_protocol_to_string_value(
    handler: &WebUIHandler,
    protocol: &HandlerProtocol,
    state: &Value,
    options: &WasmRenderOptions,
) -> Result<String, WasmError> {
    let mut writer = StringWriter::with_capacity(4096);
    handler.render(
        protocol,
        state,
        &RenderOptions::new(&options.entry, &options.request_path),
        &mut writer,
    )?;
    Ok(writer.content)
}

fn render_protocol_to_callback_value(
    handler: &WebUIHandler,
    protocol: &HandlerProtocol,
    state: &Value,
    options: &WasmRenderOptions,
    on_chunk: &Function,
) -> Result<(), WasmError> {
    let mut writer = CallbackWriter::new(on_chunk);
    handler.render(
        protocol,
        state,
        &RenderOptions::new(&options.entry, &options.request_path),
        &mut writer,
    )?;
    writer.flush()?;
    Ok(())
}

pub(crate) fn parse_optional_plugin(
    plugin: Option<&str>,
) -> Result<Option<HandlerPluginKind>, WasmError> {
    plugin.map(HandlerPluginKind::parse).transpose()
}

fn parse_render_options(options: Option<Object>) -> Result<WasmRenderOptions, WasmError> {
    let mut parsed = WasmRenderOptions::default();
    let Some(options) = options else {
        return Ok(parsed);
    };

    if let Some(entry) = optional_string_field(options.as_ref(), "entry")? {
        parsed.entry = entry;
    }
    if let Some(request_path) = optional_string_field(options.as_ref(), "requestPath")? {
        parsed.request_path = request_path;
    }
    Ok(parsed)
}

fn optional_string_field(options: &JsValue, field: &str) -> Result<Option<String>, WasmError> {
    let value = Reflect::get(options, &JsValue::from_str(field)).map_err(|_| {
        WasmError::InvalidOptions(format!("failed to read `{field}` from options object"))
    })?;
    if value.is_null() || value.is_undefined() {
        return Ok(None);
    }
    value.as_string().map(Some).ok_or_else(|| {
        WasmError::InvalidOptions(format!("`{field}` must be a string when provided"))
    })
}

fn create_handler(plugin: Option<HandlerPluginKind>) -> WebUIHandler {
    match plugin {
        Some(HandlerPluginKind::FastV2) => {
            WebUIHandler::with_plugin(|| Box::new(FastV2HydrationPlugin::new()))
        }
        Some(HandlerPluginKind::FastV3) => {
            WebUIHandler::with_plugin(|| Box::new(FastV3HydrationPlugin::new()))
        }
        Some(HandlerPluginKind::WebUI) => {
            WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()))
        }
        None => WebUIHandler::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn structural_fragment(value: &str) -> webui_protocol::WebUIFragment {
        let mut token = String::with_capacity("}}}webui:".len() + value.len());
        token.push_str("}}}webui:");
        token.push_str(value);
        webui_protocol::WebUIFragment::signal(token, true)
    }

    #[test]
    fn parse_plugin_keeps_fast_aliases_parser_free() {
        assert_eq!(
            parse_optional_plugin(Some("fast")).unwrap(),
            Some(HandlerPluginKind::FastV2)
        );
        assert_eq!(
            parse_optional_plugin(Some("fast-v2")).unwrap(),
            Some(HandlerPluginKind::FastV2)
        );
        assert_eq!(
            parse_optional_plugin(Some("fast-v3")).unwrap(),
            Some(HandlerPluginKind::FastV3)
        );
        assert_eq!(
            parse_optional_plugin(Some("webui")).unwrap(),
            Some(HandlerPluginKind::WebUI)
        );
    }

    #[test]
    fn parse_plugin_rejects_unknown_names() {
        let err = parse_optional_plugin(Some("unknown")).unwrap_err();
        assert_eq!(
            err.to_string(),
            "Unknown plugin: unknown. Use \"webui\", \"fast-v3\", \"fast-v2\", or \"fast\"."
        );
    }

    #[test]
    fn protocol_reuses_decoded_protocol() {
        use std::collections::HashMap;
        use webui_protocol::{FragmentList, WebUIFragment};

        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::signal("name".to_string(), true)],
            },
        );
        let bytes = WebUIProtocol::new(fragments)
            .to_protobuf()
            .expect("protocol should serialize");
        let protocol = Protocol::new(&bytes, None).expect("protocol should load");

        let first = protocol
            .render(r#"{"name":"first"}"#, None)
            .expect("first render should succeed");
        let second = protocol
            .render(r#"{"name":"second"}"#, None)
            .expect("second render should succeed");

        assert_eq!(first, "first");
        assert_eq!(second, "second");
    }

    #[test]
    fn render_projects_state_to_component_hydration_keys() {
        use std::collections::HashMap;
        use webui_protocol::{
            ComponentData, FragmentList, InitialStateStrategy, StateProjectionMode, WebUIFragment,
        };

        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>"),
                    structural_fragment("head_end"),
                    WebUIFragment::raw("</head><body>"),
                    WebUIFragment::component("client-card"),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>"),
                ],
            },
        );
        fragments.insert(
            "client-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>client</p>")],
            },
        );
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.initial_state_strategy = InitialStateStrategy::Components as i32;
        protocol.components.insert(
            "client-card".to_string(),
            ComponentData {
                hydration_mode: StateProjectionMode::Keys as i32,
                hydration_keys: vec!["kept".to_string()],
                ..Default::default()
            },
        );

        let rendered = render_protocol_to_string(
            &protocol,
            r#"{"kept":"KEPT_VALUE_WASM","dropped":"DROPPED_VALUE_WASM"}"#,
            "index.html",
            "/",
            Some(HandlerPluginKind::WebUI),
        )
        .expect("render should succeed");

        // Only the hydratable key reaches the bootstrap state block...
        assert!(
            rendered.contains(r#""kept":"KEPT_VALUE_WASM""#),
            "hydratable key missing from bootstrap state:\n{rendered}"
        );
        // ...the non-hydratable key is projected out entirely.
        assert!(
            !rendered.contains("DROPPED_VALUE_WASM"),
            "server-only value leaked into render:\n{rendered}"
        );
        assert!(
            !rendered.contains("dropped"),
            "server-only key name leaked into render:\n{rendered}"
        );
    }
}
