// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Parser-only WASM exports.

use crate::error::WasmError;
use std::collections::HashMap;
use wasm_bindgen::prelude::*;
use webui_parser::plugin::webui::WebUIParserPlugin;
use webui_parser::plugin::{ParserPluginArtifacts, StateSurface};
use webui_parser::{CssStrategy, HtmlParser};
use webui_protocol::projection_manifest::{ProjectionComponent, ProjectionManifest};
use webui_protocol::{InitialStateStrategy, StateProjectionMode, WebUIProtocol};

/// Build protocol protobuf bytes from virtual files without rendering.
///
/// Returns the serialized `WebUIProtocol` as protobuf bytes.
#[wasm_bindgen]
pub fn build_protocol(
    files: JsValue,
    entry: &str,
    projection_manifests: Option<JsValue>,
) -> Result<Vec<u8>, JsValue> {
    let files_map: HashMap<String, String> =
        serde_wasm_bindgen::from_value(files).map_err(|e| JsValue::from_str(&e.to_string()))?;
    let manifests: Vec<ProjectionManifest> = projection_manifests
        .map(serde_wasm_bindgen::from_value)
        .transpose()
        .map_err(|error| JsValue::from_str(&format!("invalid projection manifests: {error}")))?
        .unwrap_or_default();

    build_protocol_inner(&files_map, entry, &manifests)
        .map_err(|e| JsValue::from_str(&e.to_string()))
}

pub(crate) fn build_protocol_inner(
    files: &HashMap<String, String>,
    entry: &str,
    projection_manifests: &[ProjectionManifest],
) -> Result<Vec<u8>, WasmError> {
    let protocol = parse_to_protocol(files, entry, projection_manifests)?;
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
                // A sibling module marks an authored client component. Rust
                // never analyzes its source; optional projection manifests
                // provide exact client state surfaces.
                let script = component_script(files, tag_name);
                parser.component_registry_mut().register_component(
                    webui_parser::ComponentRegistration {
                        tag_name,
                        html_content: content,
                        css_content: css,
                        is_client_owned: script.is_some(),
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
/// source text is not analyzed by Rust.
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
    projection_manifests: &[ProjectionManifest],
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
    let projection = merge_projection_manifests(projection_manifests)?;
    protocol.initial_state_strategy = if projection.is_some() {
        InitialStateStrategy::Components as i32
    } else {
        InitialStateStrategy::Full as i32
    };
    if let Some(entries) = &projection {
        let mut missing = Vec::new();
        for artifact in &templates {
            if artifact.is_scripted
                && protocol.fragments.contains_key(&artifact.tag_name)
                && !entries.contains_key(&artifact.tag_name)
            {
                missing.push(artifact.tag_name.as_str());
            }
        }
        if !missing.is_empty() {
            missing.sort_unstable();
            return Err(WasmError::Projection(format!(
                "PROJ-B001: scripted components have no projection entry: {}",
                missing.join(", ")
            )));
        }
    }
    for artifact in templates {
        let manifest_entry = if artifact.is_scripted {
            projection
                .as_ref()
                .and_then(|entries| entries.get(&artifact.tag_name))
        } else {
            None
        };
        let component = protocol.components.entry(artifact.tag_name).or_default();
        component.template = artifact.template;
        component.template_json = artifact.template_json;
        component.template_functions = artifact.template_functions;
        let hydration = manifest_entry.map_or(artifact.hydration, |entry| {
            StateSurface::Keys(entry.hydration_keys.clone())
        });
        let navigation = manifest_entry.map_or(artifact.navigation, |entry| {
            StateSurface::Keys(union_keys(&entry.navigation_keys, &artifact.template_roots))
        });
        let (hydration_mode, hydration_keys) = encode_state_surface(hydration);
        component.hydration_mode = hydration_mode;
        component.hydration_keys = hydration_keys;
        let (navigation_mode, navigation_keys) = encode_state_surface(navigation);
        component.navigation_mode = navigation_mode;
        component.navigation_keys = navigation_keys;
    }

    fn merge_projection_manifests(
        manifests: &[ProjectionManifest],
    ) -> Result<Option<std::collections::BTreeMap<String, ProjectionComponent>>, WasmError> {
        if manifests.is_empty() {
            return Ok(None);
        }
        let mut components = std::collections::BTreeMap::new();
        for manifest in manifests {
            let serialized_size = serde_json::to_vec(manifest)
                .map_err(|error| WasmError::Projection(format!("PROJ-M009: {error}")))?
                .len();
            if serialized_size > 16 * 1024 * 1024 {
                return Err(WasmError::Projection(
                    "PROJ-S001: projection manifest exceeds the 16 MiB limit".to_string(),
                ));
            }
            manifest
                .validate()
                .map_err(|error| WasmError::Projection(format!("{}: {error}", error.code())))?;
            for (tag, entry) in &manifest.components {
                if components.insert(tag.clone(), entry.clone()).is_some() {
                    return Err(WasmError::Projection(format!(
                        "PROJ-M006: component <{tag}> is declared by more than one projection manifest"
                    )));
                }
            }
        }
        Ok(Some(components))
    }

    fn union_keys(left: &[String], right: &[String]) -> Vec<String> {
        let mut keys = Vec::with_capacity(left.len() + right.len());
        keys.extend_from_slice(left);
        keys.extend_from_slice(right);
        keys.sort_unstable();
        keys.dedup();
        keys
    }
    Ok(protocol)
}

fn encode_state_surface(surface: StateSurface) -> (i32, Vec<String>) {
    match surface {
        StateSurface::None => (StateProjectionMode::None as i32, Vec::new()),
        StateSurface::Keys(keys) => (StateProjectionMode::Keys as i32, keys),
        StateSurface::All => (StateProjectionMode::All as i32, Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_protocol_reports_missing_entry() {
        let files = HashMap::new();
        let err = build_protocol_inner(&files, "index.html", &[]).unwrap_err();
        assert_eq!(err.to_string(), "Entry file 'index.html' not found");
    }

    #[test]
    fn parse_to_protocol_without_manifest_preserves_full_state() {
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

        let protocol = parse_to_protocol(&files, "index.html", &[]).unwrap();
        let component = protocol.components.get("my-card").unwrap();

        assert_eq!(
            protocol.initial_state_strategy,
            InitialStateStrategy::Full as i32
        );
        assert_eq!(component.hydration_mode, StateProjectionMode::All as i32);
        assert!(component.hydration_keys.is_empty());
        assert!(!component.template_json.is_empty());
    }

    #[test]
    fn parse_to_protocol_applies_manifest_surfaces() {
        use std::collections::BTreeMap;
        use webui_protocol::projection_manifest::{
            ProjectionAdapter, ProjectionComponent, ProjectionProducer, PRODUCER_NAME, SCHEMA_ID,
        };

        let files = HashMap::from([
            (
                "index.html".to_string(),
                "<html><body><my-card></my-card></body></html>".to_string(),
            ),
            (
                "my-card.html".to_string(),
                "<template shadowrootmode=\"open\"><p>{{name}}</p></template>".to_string(),
            ),
            ("my-card.ts".to_string(), "export {};".to_string()),
        ]);
        let mut manifest = ProjectionManifest {
            schema: SCHEMA_ID.to_string(),
            producer: ProjectionProducer {
                name: PRODUCER_NAME.to_string(),
                version: "0.0.18".to_string(),
            },
            adapter: ProjectionAdapter {
                name: "test".to_string(),
                bundler: "test@1.0.0".to_string(),
            },
            root: ".".to_string(),
            analysis_hash: format!("sha256:{}", "1".repeat(64)),
            build_id: String::new(),
            inputs: BTreeMap::from([(
                "my-card.ts".to_string(),
                format!("sha256:{}", "2".repeat(64)),
            )]),
            outputs: BTreeMap::from([(
                "bundle.js".to_string(),
                format!("sha256:{}", "3".repeat(64)),
            )]),
            components: BTreeMap::from([(
                "my-card".to_string(),
                ProjectionComponent {
                    module: "my-card.ts".to_string(),
                    outputs: vec!["bundle.js".to_string()],
                    hydration_keys: vec!["name".to_string()],
                    navigation_keys: vec!["label".to_string(), "name".to_string()],
                },
            )]),
        };
        manifest.build_id = manifest.compute_build_id();

        let mut missing = manifest.clone();
        missing.components.clear();
        missing.build_id = missing.compute_build_id();
        let error = parse_to_protocol(&files, "index.html", &[missing]).unwrap_err();
        assert!(error.to_string().contains("PROJ-B001"));

        let protocol = parse_to_protocol(&files, "index.html", &[manifest]).unwrap();
        let component = protocol.components.get("my-card").unwrap();
        assert_eq!(
            protocol.initial_state_strategy,
            InitialStateStrategy::Components as i32
        );
        assert_eq!(component.hydration_keys, ["name"]);
        assert_eq!(component.navigation_keys, ["label", "name"]);
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

        let protocol = parse_to_protocol(&files, "index.html", &[]).unwrap();
        let component = protocol.components.get("my-card").unwrap();
        assert!(component.hydration_keys.is_empty());
        assert_eq!(component.navigation_keys, ["name"]);
        assert!(component.template_json.contains(r#""th":1"#));
    }
}
