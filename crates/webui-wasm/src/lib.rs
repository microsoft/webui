// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! WebAssembly bindings for the WebUI framework.
//!
//! This crate exposes the WebUI rendering pipeline to JavaScript via `wasm-bindgen`,
//! powering the interactive playground in the documentation site.
//!
//! Two modes of operation:
//! - **`render`** — Takes a pre-built protocol (JSON) + state and renders HTML.
//! - **`build_and_render`** — Takes virtual files + state, parses and renders HTML
//!   using the real `webui-parser` (same parser used by the CLI).

use serde_json::Value;
use std::collections::HashMap;
use wasm_bindgen::prelude::*;
use webui_handler::plugin::fast::FastHydrationPlugin;
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::{RenderOptions, ResponseWriter, WebUIHandler};
use webui_parser::{CssStrategy, HtmlParser, Plugin};
use webui_protocol::WebUIProtocol;

/// A simple string buffer for collecting rendered output.
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

/// Render a pre-built WebUI protocol with state data.
///
/// # Arguments
///
/// * `protocol_json` — JSON string of the serialized `WebUIProtocol`.
/// * `state_json` — JSON string of the state data.
/// * `plugin` — Optional plugin identifier.
///
/// # Returns
///
/// The rendered HTML string, or throws a JS error on failure.
#[wasm_bindgen]
pub fn render(
    protocol_json: &str,
    state_json: &str,
    entry: &str,
    request_path: &str,
    plugin: Option<String>,
) -> Result<String, JsValue> {
    let plugin = plugin
        .map(|s| s.parse::<Plugin>())
        .transpose()
        .map_err(|e| JsValue::from_str(&e))?;
    render_inner(protocol_json, state_json, entry, request_path, plugin)
        .map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Build and render a WebUI application from virtual files.
///
/// Uses a lightweight pure-Rust parser suitable for the playground.
/// Handles signals, for-loops, if-conditions, components, and dynamic attributes.
///
/// # Arguments
///
/// * `files` — A JS object mapping filenames to their string content.
///   Example: `{ "index.html": "<h1>{{title}}</h1>", "my-card.html": "<p><slot></slot></p>" }`
/// * `state_json` — A JSON string of the state data to render with.
/// * `entry` — The entry HTML filename (e.g. `"index.html"`).
///
/// # Returns
///
/// The rendered HTML string, or throws a JS error on failure.
#[wasm_bindgen]
pub fn build_and_render(
    files: JsValue,
    state_json: &str,
    entry: &str,
    request_path: &str,
) -> Result<String, JsValue> {
    let files_map: HashMap<String, String> =
        serde_wasm_bindgen::from_value(files).map_err(|e| JsValue::from_str(&e.to_string()))?;

    build_and_render_inner(&files_map, state_json, entry, request_path)
        .map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Build the protocol JSON from virtual files without rendering.
///
/// Returns the serialized `WebUIProtocol` as a JSON string.
#[wasm_bindgen]
pub fn build_protocol(files: JsValue, entry: &str) -> Result<String, JsValue> {
    let files_map: HashMap<String, String> =
        serde_wasm_bindgen::from_value(files).map_err(|e| JsValue::from_str(&e.to_string()))?;

    build_protocol_inner(&files_map, entry).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Produce a complete JSON partial response for client-side navigation.
///
/// Combines application state, route templates, inventory, request path, and
/// matched route chain into a single JSON string:
/// `{"state":{...},"templates":[...],"inventory":"...","path":"...","chain":[...]}`.
///
/// Host servers return this directly — no assembly required.
#[wasm_bindgen]
pub fn render_partial(
    protocol_json: &str,
    state_json: &str,
    entry_id: &str,
    request_path: &str,
    inventory_hex: &str,
) -> Result<String, JsValue> {
    let protocol: WebUIProtocol = serde_json::from_str(protocol_json)
        .map_err(|e| JsValue::from_str(&format!("Protocol JSON error: {e}")))?;

    let state: serde_json::Value = serde_json::from_str(state_json)
        .map_err(|e| JsValue::from_str(&format!("invalid state JSON: {e}")))?;

    // TODO: ProtocolIndex is created per-request here. Ideally the host should
    // cache it alongside the protocol — it's deterministic per protocol.
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

#[wasm_bindgen]
pub fn render_component_templates(
    protocol_json: &str,
    component_tags_json: &str,
    inventory_hex: &str,
) -> Result<String, JsValue> {
    let protocol: WebUIProtocol = serde_json::from_str(protocol_json)
        .map_err(|e| JsValue::from_str(&format!("Protocol JSON error: {e}")))?;

    let tags: Vec<String> = serde_json::from_str(component_tags_json)
        .map_err(|e| JsValue::from_str(&format!("invalid tags JSON: {e}")))?;
    let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();

    // Per-request index — see ProtocolIndex doc for caching guidance.
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

fn build_protocol_inner(
    files: &HashMap<String, String>,
    entry: &str,
) -> Result<String, BuildError> {
    let protocol = parse_to_protocol(files, entry)?;
    serde_json::to_string(&protocol).map_err(BuildError::Protocol)
}

/// Create a handler with an optional plugin.
fn create_handler(plugin: Option<Plugin>) -> Result<WebUIHandler, BuildError> {
    match plugin {
        Some(Plugin::Fast) => Ok(WebUIHandler::with_plugin(|| {
            Box::new(FastHydrationPlugin::new())
        })),
        Some(Plugin::WebUI) => Ok(WebUIHandler::with_plugin(|| {
            Box::new(WebUIHydrationPlugin::new())
        })),
        None => Ok(WebUIHandler::new()),
    }
}

fn render_inner(
    protocol_json: &str,
    state_json: &str,
    entry: &str,
    request_path: &str,
    plugin: Option<Plugin>,
) -> Result<String, BuildError> {
    let protocol: WebUIProtocol =
        serde_json::from_str(protocol_json).map_err(BuildError::Protocol)?;
    let state: Value = serde_json::from_str(state_json).map_err(BuildError::State)?;

    let mut writer = StringWriter::with_capacity(1024);
    let handler = create_handler(plugin)?;
    handler.render(
        &protocol,
        &state,
        &RenderOptions::new(entry, request_path),
        &mut writer,
    )?;

    Ok(writer.content)
}

/// Core build-and-render implementation (testable without WASM).
pub(crate) fn build_and_render_inner(
    files: &HashMap<String, String>,
    state_json: &str,
    entry: &str,
    request_path: &str,
) -> Result<String, BuildError> {
    let protocol = parse_to_protocol(files, entry)?;

    let state: Value = serde_json::from_str(state_json).map_err(BuildError::State)?;

    let mut writer = StringWriter::with_capacity(1024);
    let handler = create_handler(None)?;
    handler.render(
        &protocol,
        &state,
        &RenderOptions::new(entry, request_path),
        &mut writer,
    )?;

    Ok(writer.content)
}

/// Parse virtual files into a `WebUIProtocol` using the real `webui-parser`.
fn parse_to_protocol(
    files: &HashMap<String, String>,
    entry: &str,
) -> Result<WebUIProtocol, BuildError> {
    let entry_html = files
        .get(entry)
        .ok_or_else(|| BuildError::MissingEntry(entry.to_string()))?;

    let mut parser = HtmlParser::new();
    parser.set_css_strategy(CssStrategy::Style);

    // Register components from virtual files (no filesystem needed)
    for (filename, content) in files {
        if filename != entry && filename.ends_with(".html") {
            let tag_name = filename.trim_end_matches(".html");
            if tag_name.contains('-') {
                let css_key = format!("{tag_name}.css");
                let css = files.get(&css_key).map(|s| s.as_str());
                parser
                    .component_registry_mut()
                    .register_component(tag_name, content, css)?;
            }
        }
    }

    parser.parse(entry, entry_html)?;

    Ok(WebUIProtocol::new(parser.into_fragment_records()))
}

/// Errors from the build-and-render pipeline.
#[derive(Debug, thiserror::Error)]
pub(crate) enum BuildError {
    #[error("Entry file '{0}' not found")]
    MissingEntry(String),

    #[error("{0}")]
    Parse(#[from] webui_parser::ParserError),

    #[error("Protocol JSON error: {0}")]
    Protocol(serde_json::Error),

    #[error("State JSON error: {0}")]
    State(serde_json::Error),

    #[error("{0}")]
    Render(#[from] webui_handler::HandlerError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_render() {
        let mut files = HashMap::new();
        files.insert(
            "index.html".to_string(),
            "<h1>Hello, {{name}}!</h1>".to_string(),
        );

        let result = build_and_render_inner(&files, r#"{"name": "WebUI"}"#, "index.html", "/");
        assert_eq!(result.unwrap(), "<h1>Hello, WebUI!</h1>");
    }

    #[test]
    fn test_missing_entry_file() {
        let files = HashMap::new();
        let result = build_and_render_inner(&files, "{}", "index.html", "/");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"), "Unexpected error: {}", err);
    }

    #[test]
    fn test_with_component() {
        let mut files = HashMap::new();
        files.insert(
            "index.html".to_string(),
            "<my-card>World</my-card>".to_string(),
        );
        files.insert(
            "my-card.html".to_string(),
            "<div class=\"card\"><slot></slot></div>".to_string(),
        );

        let result = build_and_render_inner(&files, "{}", "index.html", "/");
        assert!(result.is_ok(), "Render failed: {:?}", result);
        let html = result.as_deref().unwrap_or("");
        assert!(html.contains("card"), "Expected card class in: {}", html);
    }

    #[test]
    fn test_with_for_loop() {
        let mut files = HashMap::new();
        files.insert(
            "index.html".to_string(),
            "<for each=\"item in items\">{{item.name}}, </for>".to_string(),
        );

        let state = r#"{"items": [{"name": "A"}, {"name": "B"}]}"#;
        let result = build_and_render_inner(&files, state, "index.html", "/");
        assert!(result.is_ok(), "Render failed: {:?}", result);
        let html = result.as_deref().unwrap_or("");
        assert!(html.contains("A"), "Expected 'A' in: {}", html);
        assert!(html.contains("B"), "Expected 'B' in: {}", html);
    }

    #[test]
    fn test_with_if_condition() {
        let mut files = HashMap::new();
        files.insert(
            "index.html".to_string(),
            "<if condition=\"show\">Visible</if>".to_string(),
        );

        let result_true = build_and_render_inner(&files, r#"{"show": true}"#, "index.html", "/");
        assert_eq!(result_true.unwrap(), "Visible");

        let result_false = build_and_render_inner(&files, r#"{"show": false}"#, "index.html", "/");
        assert_eq!(result_false.unwrap(), "");
    }

    #[test]
    fn test_invalid_state_json() {
        let mut files = HashMap::new();
        files.insert("index.html".to_string(), "<p>Hi</p>".to_string());

        let result = build_and_render_inner(&files, "not json", "index.html", "/");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("State JSON error"),
            "Unexpected error: {}",
            err
        );
    }

    #[test]
    fn test_component_with_css() {
        let mut files = HashMap::new();
        files.insert(
            "index.html".to_string(),
            "<my-card>Content</my-card>".to_string(),
        );
        files.insert(
            "my-card.html".to_string(),
            "<p><slot></slot></p>".to_string(),
        );
        files.insert("my-card.css".to_string(), "p { color: red; }".to_string());

        let result = build_and_render_inner(&files, "{}", "index.html", "/");
        assert!(result.is_ok(), "Render failed: {:?}", result);
        let html = result.as_deref().unwrap_or("");
        // WASM uses CssStrategy::Style, so CSS should be in <style> tags, not <link>
        assert!(
            html.contains("<style>p { color: red; }</style>"),
            "Expected inline <style> tag in: {}",
            html
        );
        assert!(
            !html.contains("<link"),
            "Should not have external <link> tag in: {}",
            html
        );
    }

    #[test]
    fn test_raw_signal() {
        let mut files = HashMap::new();
        files.insert(
            "index.html".to_string(),
            "<div>{{{raw_html}}}</div>".to_string(),
        );

        let result =
            build_and_render_inner(&files, r#"{"raw_html": "<b>bold</b>"}"#, "index.html", "/");
        assert!(result.is_ok(), "Render failed: {:?}", result);
        let html = result.as_deref().unwrap_or("");
        assert!(
            html.contains("<b>bold</b>"),
            "Expected raw HTML in: {}",
            html
        );
    }

    #[test]
    fn test_static_html_passthrough() {
        let mut files = HashMap::new();
        files.insert(
            "index.html".to_string(),
            "<h1>Static</h1><p>Content</p>".to_string(),
        );

        let result = build_and_render_inner(&files, "{}", "index.html", "/");
        assert_eq!(result.unwrap(), "<h1>Static</h1><p>Content</p>");
    }
}
