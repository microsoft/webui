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
use webui_handler::{HandlerError, RenderOptions, ResponseWriter, WebUIHandler};
use webui_protocol::WebUIProtocol;

/// A simple string buffer for collecting rendered output.
#[cfg(test)]
struct StringWriter {
    content: String,
}

#[cfg(test)]
impl StringWriter {
    fn with_capacity(cap: usize) -> Self {
        Self {
            content: String::with_capacity(cap),
        }
    }
}

#[cfg(test)]
impl ResponseWriter for StringWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.content.push_str(content);
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

/// A writer that streams rendered fragments to a JavaScript callback.
struct CallbackWriter<'a> {
    on_chunk: &'a Function,
}

impl<'a> CallbackWriter<'a> {
    fn new(on_chunk: &'a Function) -> Self {
        Self { on_chunk }
    }
}

impl ResponseWriter for CallbackWriter<'_> {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.on_chunk
            .call1(&JsValue::UNDEFINED, &JsValue::from_str(content))
            .map(|_| ())
            .map_err(|e| HandlerError::Writer(format!("{e:?}")))
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
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

/// Render a pre-built WebUI protocol with state data, streaming chunks to a callback.
///
/// # Arguments
///
/// * `protocol_bytes` - Protobuf bytes of the serialized `WebUIProtocol`.
/// * `state_json` - JSON string of the state data.
/// * `on_chunk` - Callback invoked with each rendered HTML fragment.
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
/// Combines application state, route templates, inventory, request path, and
/// matched route chain into a single JSON string:
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
    let protocol = WebUIProtocol::from_protobuf(protocol_bytes)
        .map_err(|e| JsValue::from_str(&format!("Protocol error: {e}")))?;

    let state: serde_json::Value = serde_json::from_str(state_json)
        .map_err(|e| JsValue::from_str(&format!("invalid state JSON: {e}")))?;

    let mut index = webui_handler::route_handler::ProtocolIndex::new(&protocol);

    let mut result = webui_handler::route_handler::render_partial(
        &protocol,
        entry_id,
        request_path,
        inventory_hex,
        &mut index,
    )
    .map_err(|e| JsValue::from_str(&format!("render_partial failed: {e}")))?;
    if let Some(obj) = result.as_object_mut() {
        obj.insert("state".into(), state);
    }

    serde_json::to_string(&result)
        .map_err(|e| JsValue::from_str(&format!("JSON serialize error: {e}")))
}

/// Extract the CSS token name list from protocol protobuf bytes.
///
/// Returns a JavaScript array of token name strings, preserving the original
/// order from the build step.
#[wasm_bindgen]
pub fn protocol_tokens(protocol_bytes: &[u8]) -> Result<JsValue, JsValue> {
    let protocol = WebUIProtocol::from_protobuf(protocol_bytes)
        .map_err(|e| JsValue::from_str(&format!("Protocol error: {e}")))?;

    serde_wasm_bindgen::to_value(&protocol.tokens)
        .map_err(|e| JsValue::from_str(&format!("Serialization error: {e}")))
}

/// Return component template payloads for requested component tags.
#[wasm_bindgen]
pub fn render_component_templates(
    protocol_bytes: &[u8],
    component_tags_json: &str,
    inventory_hex: &str,
) -> Result<String, JsValue> {
    let protocol = WebUIProtocol::from_protobuf(protocol_bytes)
        .map_err(|e| JsValue::from_str(&format!("Protocol error: {e}")))?;

    let tags: Vec<String> = serde_json::from_str(component_tags_json)
        .map_err(|e| JsValue::from_str(&format!("invalid tags JSON: {e}")))?;
    let tag_refs: Vec<&str> = tags.iter().map(String::as_str).collect();

    let index = webui_handler::route_handler::ProtocolIndex::new(&protocol);

    let result = webui_handler::route_handler::render_component_templates(
        &protocol,
        &tag_refs,
        inventory_hex,
        &index,
    )
    .map_err(|e| JsValue::from_str(&format!("render_component_templates failed: {e}")))?;

    serde_json::to_string(&result)
        .map_err(|e| JsValue::from_str(&format!("JSON serialize error: {e}")))
}

fn render_stream_inner(
    protocol_bytes: &[u8],
    state_json: &str,
    on_chunk: &Function,
    options: &WasmRenderOptions,
) -> Result<(), WasmError> {
    let protocol = WebUIProtocol::from_protobuf(protocol_bytes)?;
    render_protocol_to_callback(&protocol, state_json, options, on_chunk)
}

#[cfg(test)]
pub(crate) fn render_protocol_to_string(
    protocol: &WebUIProtocol,
    state_json: &str,
    entry: &str,
    request_path: &str,
    plugin: Option<HandlerPluginKind>,
) -> Result<String, WasmError> {
    let state: Value = serde_json::from_str(state_json).map_err(WasmError::State)?;

    let mut writer = StringWriter::with_capacity(1024);
    let handler = create_handler(plugin);
    handler.render(
        protocol,
        &state,
        &RenderOptions::new(entry, request_path),
        &mut writer,
    )?;

    Ok(writer.content)
}

fn render_protocol_to_callback(
    protocol: &WebUIProtocol,
    state_json: &str,
    options: &WasmRenderOptions,
    on_chunk: &Function,
) -> Result<(), WasmError> {
    let state: Value = serde_json::from_str(state_json).map_err(WasmError::State)?;

    let mut writer = CallbackWriter::new(on_chunk);
    let handler = create_handler(options.plugin);
    handler.render(
        protocol,
        &state,
        &RenderOptions::new(&options.entry, &options.request_path),
        &mut writer,
    )?;

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
}
