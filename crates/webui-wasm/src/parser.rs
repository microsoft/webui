// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Parser-only WASM exports.

use crate::error::WasmError;
use std::collections::HashMap;
use wasm_bindgen::prelude::*;
use webui_parser::plugin::webui::WebUIParserPlugin;
use webui_parser::{CssStrategy, HtmlParser};
use webui_protocol::WebUIProtocol;

/// Build protocol protobuf bytes from virtual files without rendering.
///
/// Returns the serialized `WebUIProtocol` as protobuf bytes.
#[wasm_bindgen]
pub fn build_protocol(files: JsValue, entry: &str) -> Result<Vec<u8>, JsValue> {
    let files_map: HashMap<String, String> =
        serde_wasm_bindgen::from_value(files).map_err(|e| JsValue::from_str(&e.to_string()))?;

    build_protocol_inner(&files_map, entry).map_err(|e| JsValue::from_str(&e.to_string()))
}

pub(crate) fn build_protocol_inner(
    files: &HashMap<String, String>,
    entry: &str,
) -> Result<Vec<u8>, WasmError> {
    let protocol = parse_to_protocol(files, entry)?;
    protocol.to_protobuf().map_err(WasmError::Protocol)
}

/// Register all component `.html` files and optional companion `.css` files
/// from the virtual file map, skipping the entry.
fn register_components(
    parser: &mut HtmlParser,
    files: &HashMap<String, String>,
    entry: &str,
) -> Result<(), WasmError> {
    for (filename, content) in files {
        if filename != entry && filename.ends_with(".html") {
            let tag_name = filename.trim_end_matches(".html");
            if tag_name.contains('-') {
                let css_key = format!("{tag_name}.css");
                let css = files.get(&css_key).map(String::as_str);
                // The sibling module (if any) is the `has_script` signal; its raw
                // source is carried through to the WebUI parser plugin, which
                // derives the hydration surface itself. Keeping the wasm path
                // scan-free mirrors the native build's plugin-owned strategy.
                let script = component_script(files, tag_name);
                parser.component_registry_mut().register_component(
                    webui_parser::ComponentRegistration {
                        tag_name,
                        html_content: content,
                        css_content: css,
                        has_script: script.is_some(),
                        script_source: script,
                    },
                )?;
            }
        }
    }
    Ok(())
}

/// Return the authored browser module source for a component, if present.
///
/// Prefers `.ts` over `.js`. Its presence is the static-host `has_script`
/// signal; the raw source is handed to parser plugins so each can derive its
/// own hydration surface.
fn component_script<'a>(files: &'a HashMap<String, String>, tag_name: &str) -> Option<&'a str> {
    files
        .get(&format!("{tag_name}.ts"))
        .or_else(|| files.get(&format!("{tag_name}.js")))
        .map(String::as_str)
}

/// Parse virtual files into a `WebUIProtocol` using the real `webui-parser`
/// with the WebUI plugin.
pub(crate) fn parse_to_protocol(
    files: &HashMap<String, String>,
    entry: &str,
) -> Result<WebUIProtocol, WasmError> {
    let entry_html = files
        .get(entry)
        .ok_or_else(|| WasmError::MissingEntry(entry.to_string()))?;

    let mut parser =
        HtmlParser::with_plugin_options(Box::new(WebUIParserPlugin::new()), CssStrategy::Style);
    register_components(&mut parser, files, entry)?;
    parser.parse(entry, entry_html)?;
    parser.take_plugin_artifacts()?;

    Ok(WebUIProtocol::new(parser.into_fragment_records()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_protocol_reports_missing_entry() {
        let files = HashMap::new();
        let err = build_protocol_inner(&files, "index.html").unwrap_err();
        assert_eq!(err.to_string(), "Entry file 'index.html' not found");
    }
}
