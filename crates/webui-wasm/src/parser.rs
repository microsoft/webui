// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Parser-only WASM exports.

use crate::error::WasmError;
use std::collections::HashMap;
use wasm_bindgen::prelude::*;
use webui_parser::plugin::webui::WebUIParserPlugin;
use webui_parser::plugin::ParserPluginArtifacts;
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
                // A sibling module marks an authored client component. Its raw
                // source is carried through to the WebUI parser plugin, which
                // derives the hydration surface itself. Scriptless components
                // retain dormant template metadata for soft navigation.
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
/// Prefers `.ts` over `.js`. Its presence is the client-component signal; the
/// raw source is handed to parser plugins so each can derive its own hydration
/// surface.
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
    let templates = match parser.take_plugin_artifacts()? {
        ParserPluginArtifacts::None => Vec::new(),
        ParserPluginArtifacts::ComponentTemplates(templates) => templates,
    };

    let mut protocol = WebUIProtocol::new(parser.into_fragment_records());
    for artifact in templates {
        let component = protocol.components.entry(artifact.tag_name).or_default();
        component.template = artifact.template;
        component.template_json = artifact.template_json;
        component.template_functions = artifact.template_functions;
        component.hydration_keys = artifact.hydration_keys;
        component.navigation_keys = artifact.navigation_keys;
    }
    Ok(protocol)
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

    #[test]
    fn parse_to_protocol_preserves_webui_hydration_artifacts() {
        let files = HashMap::from([
            (
                "index.html".to_string(),
                "<html><body><my-card></my-card></body></html>".to_string(),
            ),
            (
                "my-card.html".to_string(),
                "<template shadowrootmode=\"open\"><p>{{name}}</p></template>".to_string(),
            ),
            (
                "my-card.ts".to_string(),
                "class MyCard { @observable name = ''; }".to_string(),
            ),
        ]);

        let protocol = parse_to_protocol(&files, "index.html").unwrap();
        let component = protocol.components.get("my-card").unwrap();

        assert_eq!(component.hydration_keys, ["name"]);
        assert!(!component.template_json.is_empty());
    }

    #[test]
    fn parse_to_protocol_keeps_scriptless_navigation_metadata() {
        let files = HashMap::from([
            (
                "index.html".to_string(),
                "<html><body><my-card></my-card></body></html>".to_string(),
            ),
            (
                "my-card.html".to_string(),
                "<template shadowrootmode=\"open\"><p>{{name}}</p></template>".to_string(),
            ),
        ]);

        let protocol = parse_to_protocol(&files, "index.html").unwrap();
        let component = protocol.components.get("my-card").unwrap();
        assert!(component.hydration_keys.is_empty());
        assert_eq!(component.navigation_keys, ["name"]);
        assert!(component.template_json.contains(r#""th":1"#));
    }
}
