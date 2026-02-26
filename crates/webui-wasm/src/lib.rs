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
use webui_handler::{ResponseWriter, WebUIHandler};
use webui_parser::{CssStrategy, HtmlParser};
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
///
/// # Returns
///
/// The rendered HTML string, or throws a JS error on failure.
#[wasm_bindgen]
pub fn render(protocol_json: &str, state_json: &str) -> Result<String, JsValue> {
    render_inner(protocol_json, state_json).map_err(|e| JsValue::from_str(&e.to_string()))
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
pub fn build_and_render(files: JsValue, state_json: &str, entry: &str) -> Result<String, JsValue> {
    let files_map: HashMap<String, String> =
        serde_wasm_bindgen::from_value(files).map_err(|e| JsValue::from_str(&e.to_string()))?;

    build_and_render_inner(&files_map, state_json, entry)
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

fn build_protocol_inner(
    files: &HashMap<String, String>,
    entry: &str,
) -> Result<String, BuildError> {
    let protocol = parse_to_protocol(files, entry)?;
    serde_json::to_string(&protocol).map_err(|e| BuildError::Protocol(e.to_string()))
}

fn render_inner(protocol_json: &str, state_json: &str) -> Result<String, BuildError> {
    let protocol: WebUIProtocol =
        serde_json::from_str(protocol_json).map_err(|e| BuildError::Protocol(e.to_string()))?;
    let state: Value =
        serde_json::from_str(state_json).map_err(|e| BuildError::State(e.to_string()))?;

    let mut writer = StringWriter::with_capacity(1024);
    let handler = WebUIHandler::new();
    handler
        .render(&protocol, &state, &mut writer)
        .map_err(|e| BuildError::Render(e.to_string()))?;

    Ok(writer.content)
}

/// Core build-and-render implementation (testable without WASM).
pub(crate) fn build_and_render_inner(
    files: &HashMap<String, String>,
    state_json: &str,
    entry: &str,
) -> Result<String, BuildError> {
    let protocol = parse_to_protocol(files, entry)?;

    let state: Value =
        serde_json::from_str(state_json).map_err(|e| BuildError::State(e.to_string()))?;

    let mut writer = StringWriter::with_capacity(1024);
    let handler = WebUIHandler::new();
    handler
        .render(&protocol, &state, &mut writer)
        .map_err(|e| BuildError::Render(e.to_string()))?;

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
    parser.set_css_strategy(CssStrategy::Inline);

    // Register components from virtual files (no filesystem needed)
    for (filename, content) in files {
        if filename != entry && filename.ends_with(".html") {
            let tag_name = filename.trim_end_matches(".html");
            if tag_name.contains('-') {
                let css_key = format!("{tag_name}.css");
                let css = files.get(&css_key).map(|s| s.as_str());
                parser
                    .component_registry_mut()
                    .register_component(tag_name, content, css)
                    .map_err(|e| BuildError::Parse(e.to_string()))?;
            }
        }
    }

    parser
        .parse(entry, entry_html)
        .map_err(|e| BuildError::Parse(e.to_string()))?;

    Ok(WebUIProtocol {
        fragments: parser.into_fragment_records(),
    })
}

/// Errors from the build-and-render pipeline.
#[derive(Debug, PartialEq)]
pub(crate) enum BuildError {
    MissingEntry(String),
    Parse(String),
    Protocol(String),
    State(String),
    Render(String),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::MissingEntry(name) => write!(f, "Entry file '{name}' not found"),
            BuildError::Parse(msg) => write!(f, "Parse error: {msg}"),
            BuildError::Protocol(msg) => write!(f, "Protocol JSON error: {msg}"),
            BuildError::State(msg) => write!(f, "State JSON error: {msg}"),
            BuildError::Render(msg) => write!(f, "Render error: {msg}"),
        }
    }
}

#[cfg(test)]
mod tests;
