// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Handler-only WASM exports.

use crate::error::WasmError;
use js_sys::{Function, Object, Reflect};
use serde_json::Value;
use wasm_bindgen::prelude::*;
use webui_handler::plugin::fast_v2::FastV2HydrationPlugin;
use webui_handler::plugin::fast_v3::FastV3HydrationPlugin;
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::{
    HandlerError, PreparedProtocol as HandlerPreparedProtocol, RenderOptions, ResponseWriter,
    WebUIHandler,
};
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
    plugin: Option<HandlerPluginKind>,
}

impl Default for WasmRenderOptions {
    fn default() -> Self {
        Self {
            entry: "index.html".to_string(),
            request_path: "/".to_string(),
            plugin: None,
        }
    }
}

/// A decoded protocol with reusable indices for repeated WASM renders.
#[wasm_bindgen]
pub struct PreparedProtocol {
    inner: HandlerPreparedProtocol,
}

#[wasm_bindgen]
impl PreparedProtocol {
    /// Decode protobuf bytes once for repeated rendering.
    #[wasm_bindgen(constructor)]
    pub fn new(protocol_bytes: &[u8]) -> Result<PreparedProtocol, JsValue> {
        HandlerPreparedProtocol::from_protobuf(protocol_bytes)
            .map(|inner| Self { inner })
            .map_err(|error| JsValue::from_str(&format!("Protocol error: {error}")))
    }

    /// Render from an existing JSON string.
    #[wasm_bindgen(js_name = renderJson)]
    pub fn render_json(
        &self,
        state_json: &str,
        options: Option<Object>,
    ) -> Result<String, JsValue> {
        let options =
            parse_render_options(options).map_err(|error| JsValue::from_str(&error.to_string()))?;
        let state =
            parse_state_json(state_json).map_err(|error| JsValue::from_str(&error.to_string()))?;
        render_protocol_to_string_value(self.inner.protocol(), &state, &options)
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// Stream from an existing JSON string in bounded chunks.
    #[wasm_bindgen(js_name = renderStreamJson)]
    pub fn render_stream_json(
        &self,
        state_json: &str,
        on_chunk: &Function,
        options: Option<Object>,
    ) -> Result<(), JsValue> {
        let options =
            parse_render_options(options).map_err(|error| JsValue::from_str(&error.to_string()))?;
        let state =
            parse_state_json(state_json).map_err(|error| JsValue::from_str(&error.to_string()))?;
        render_protocol_to_callback_value(self.inner.protocol(), &state, &options, on_chunk)
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
        render_partial_prepared(
            &self.inner,
            state_json,
            entry_id,
            request_path,
            inventory_hex,
        )
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
        render_component_templates_prepared(&self.inner, &tags, inventory_hex)
    }

    /// Return CSS token names in build order.
    #[wasm_bindgen(js_name = protocolTokens)]
    pub fn protocol_tokens(&self) -> Result<JsValue, JsValue> {
        serde_wasm_bindgen::to_value(self.inner.tokens())
            .map_err(|error| JsValue::from_str(&format!("Serialization error: {error}")))
    }
}

/// Render a pre-built WebUI protocol with state data, streaming chunks to a callback.
///
/// # Arguments
///
/// * `protocol_bytes` - Protobuf bytes of the serialized `WebUIProtocol`.
/// * `state_json` - JSON string of the state data.
/// * `on_chunk` - Callback invoked with output coalesced around a 16 KiB target.
/// * `options` - Optional object with `entry`, `requestPath`, and `plugin` fields.
///
/// # Returns
///
/// Nothing on success, or throws a JS error on failure.
#[wasm_bindgen]
pub fn render(
    protocol_bytes: &[u8],
    state_json: &str,
    on_chunk: &Function,
    options: Option<Object>,
) -> Result<(), JsValue> {
    let options = parse_render_options(options).map_err(|e| JsValue::from_str(&e.to_string()))?;
    render_stream_inner(protocol_bytes, state_json, on_chunk, &options)
        .map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Produce a complete JSON partial response for client-side navigation.
///
/// Combines active-route projected state, route templates, inventory, request
/// path, and matched route chain into a single JSON string:
/// `{"state":{...},"templates":[...],"inventory":"...","path":"...","chain":[...]}`.
///
/// Host servers return this directly - no assembly required.
#[wasm_bindgen]
pub fn render_partial(
    protocol_bytes: &[u8],
    state_json: &str,
    entry_id: &str,
    request_path: &str,
    inventory_hex: &str,
) -> Result<String, JsValue> {
    let prepared = HandlerPreparedProtocol::from_protobuf(protocol_bytes)
        .map_err(|e| JsValue::from_str(&format!("Protocol error: {e}")))?;
    render_partial_prepared(&prepared, state_json, entry_id, request_path, inventory_hex)
}

/// Extract the CSS token name list from protocol protobuf bytes.
///
/// Returns a JavaScript array of token name strings, preserving the original
/// order from the build step.
#[wasm_bindgen]
pub fn protocol_tokens(protocol_bytes: &[u8]) -> Result<JsValue, JsValue> {
    let prepared = HandlerPreparedProtocol::from_protobuf(protocol_bytes)
        .map_err(|e| JsValue::from_str(&format!("Protocol error: {e}")))?;

    serde_wasm_bindgen::to_value(prepared.tokens())
        .map_err(|e| JsValue::from_str(&format!("Serialization error: {e}")))
}

/// Return component template payloads for requested component tags.
#[wasm_bindgen]
pub fn render_component_templates(
    protocol_bytes: &[u8],
    component_tags_json: &str,
    inventory_hex: &str,
) -> Result<String, JsValue> {
    let prepared = HandlerPreparedProtocol::from_protobuf(protocol_bytes)
        .map_err(|e| JsValue::from_str(&format!("Protocol error: {e}")))?;
    let tags: Vec<String> = serde_json::from_str(component_tags_json)
        .map_err(|e| JsValue::from_str(&format!("invalid tags JSON: {e}")))?;
    render_component_templates_prepared(&prepared, &tags, inventory_hex)
}

fn render_stream_inner(
    protocol_bytes: &[u8],
    state_json: &str,
    on_chunk: &Function,
    options: &WasmRenderOptions,
) -> Result<(), WasmError> {
    let protocol = WebUIProtocol::from_protobuf(protocol_bytes)?;
    let state = parse_state_json(state_json)?;
    render_protocol_to_callback_value(&protocol, &state, options, on_chunk)
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
        plugin,
    };
    render_protocol_to_string_value(protocol, &state, &options)
}

fn parse_state_json(state_json: &str) -> Result<Value, WasmError> {
    serde_json::from_str(state_json).map_err(WasmError::State)
}

fn render_protocol_to_string_value(
    protocol: &WebUIProtocol,
    state: &Value,
    options: &WasmRenderOptions,
) -> Result<String, WasmError> {
    let mut writer = StringWriter::with_capacity(4096);
    let handler = create_handler(options.plugin);
    handler.render(
        protocol,
        state,
        &RenderOptions::new(&options.entry, &options.request_path),
        &mut writer,
    )?;
    Ok(writer.content)
}

fn render_protocol_to_callback_value(
    protocol: &WebUIProtocol,
    state: &Value,
    options: &WasmRenderOptions,
    on_chunk: &Function,
) -> Result<(), WasmError> {
    let mut writer = CallbackWriter::new(on_chunk);
    let handler = create_handler(options.plugin);
    handler.render(
        protocol,
        state,
        &RenderOptions::new(&options.entry, &options.request_path),
        &mut writer,
    )?;
    writer.flush()?;
    Ok(())
}

fn render_partial_prepared(
    prepared: &HandlerPreparedProtocol,
    state_json: &str,
    entry_id: &str,
    request_path: &str,
    inventory_hex: &str,
) -> Result<String, JsValue> {
    webui_handler::route_handler::render_partial_prepared(
        prepared,
        state_json,
        entry_id,
        request_path,
        inventory_hex,
    )
    .map_err(|error| JsValue::from_str(&format!("render_partial failed: {error}")))
}

fn render_component_templates_prepared(
    prepared: &HandlerPreparedProtocol,
    tags: &[String],
    inventory_hex: &str,
) -> Result<String, JsValue> {
    let tag_refs: Vec<&str> = tags.iter().map(String::as_str).collect();
    let result = webui_handler::route_handler::render_component_templates_prepared(
        prepared,
        &tag_refs,
        inventory_hex,
    )
    .map_err(|error| JsValue::from_str(&format!("render_component_templates failed: {error}")))?;

    serde_json::to_string(&result)
        .map_err(|error| JsValue::from_str(&format!("JSON serialize error: {error}")))
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
    let plugin = optional_string_field(options.as_ref(), "plugin")?;
    parsed.plugin = parse_optional_plugin(plugin.as_deref())?;

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
    fn prepared_protocol_reuses_decoded_protocol() {
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
        let prepared = PreparedProtocol::new(&bytes).expect("protocol should prepare");

        let first = prepared
            .render_json(r#"{"name":"first"}"#, None)
            .expect("first render should succeed");
        let second = prepared
            .render_json(r#"{"name":"second"}"#, None)
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
                    WebUIFragment::signal("head_end".to_string(), true),
                    WebUIFragment::raw("</head><body>"),
                    WebUIFragment::component("client-card"),
                    WebUIFragment::signal("body_end".to_string(), true),
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
