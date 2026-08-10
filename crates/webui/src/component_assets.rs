// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Static component asset graph rendering for CDN-loadable ESM modules.

mod graph;
mod json;
mod metafile;
mod payload;
mod render;
mod serialize;
mod traversal;

use std::collections::HashSet;
use webui_protocol::WebUIProtocol;

use crate::{AssetFileNameTemplate, WebUIError};

/// A rendered static component asset root or shared chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentAssetFile {
    /// Output filename for the ESM asset.
    pub name: String,
    /// JavaScript module content.
    pub content: String,
}

/// Rendered component asset graph.
#[derive(Debug)]
pub struct ComponentAssetGraph {
    /// Root and shared chunk ESM files.
    pub files: Vec<ComponentAssetFile>,
    /// Optional esbuild-compatible metafile JSON.
    pub metafile: Option<String>,
    entry_fragments: Vec<String>,
    entry_components: Vec<String>,
}

impl ComponentAssetGraph {
    pub(crate) fn retain_entry_protocol(&self, protocol: &mut WebUIProtocol) {
        if self.files.is_empty() {
            return;
        }
        protocol
            .fragments
            .retain(|name, _| self.entry_fragments.binary_search(name).is_ok());
        protocol
            .components
            .retain(|name, _| self.entry_components.binary_search(name).is_ok());
        let fragments = &protocol.fragments;
        protocol
            .style_closures
            .retain(|name, _| fragments.contains_key(name));
        let components = &protocol.components;
        for closure in protocol.style_closures.values_mut() {
            closure
                .component_tags
                .retain(|name| components.contains_key(name));
        }
    }
}

/// Render a static component asset graph.
///
/// Entry-reachable components remain external prerequisites. Components used
/// by one requested root stay inline, while components with an identical
/// multi-root consumer set are emitted once in a shared chunk.
///
/// # Errors
///
/// Returns [`WebUIError`] when the entry/root graph is invalid, contains a
/// route, lacks compiled template metadata, or produces colliding filenames.
#[must_use = "component asset graph files must be written or otherwise consumed"]
pub fn render_component_assets(
    protocol: &WebUIProtocol,
    entry: &str,
    roots: &[String],
    file_name_template: &str,
    emit_metafile: bool,
) -> Result<ComponentAssetGraph, WebUIError> {
    if roots.is_empty() {
        if emit_metafile {
            return Err(WebUIError::InvalidBuildOptions(
                "metafile requires at least one component_asset_root".to_string(),
            ));
        }
        return Ok(ComponentAssetGraph {
            files: Vec::new(),
            metafile: None,
            entry_fragments: Vec::new(),
            entry_components: Vec::new(),
        });
    }

    let file_name_template =
        AssetFileNameTemplate::try_new(file_name_template.to_string(), "asset_file_name_template")
            .map_err(|error| WebUIError::InvalidBuildOptions(error.to_string()))?;
    let plan = graph::plan_component_assets(protocol, entry, roots)?;
    let rendered =
        render::render_component_asset_graph(protocol, &plan, &file_name_template, emit_metafile)?;
    validate_unique_asset_file_names(&rendered.files)?;
    let metafile = if emit_metafile {
        Some(metafile::render_metafile(protocol, &rendered.outputs)?)
    } else {
        None
    };

    Ok(ComponentAssetGraph {
        files: rendered.files,
        metafile,
        entry_fragments: plan.entry_fragments,
        entry_components: plan.entry_components,
    })
}

fn validate_unique_asset_file_names(files: &[ComponentAssetFile]) -> Result<(), WebUIError> {
    let mut names = HashSet::with_capacity(files.len());
    for file in files {
        if !names.insert(file.name.as_str()) {
            return Err(WebUIError::InvalidBuildOptions(format!(
                "component asset filename collision for '{}'. Adjust --asset-file-name-template to include [name] or another unique component-specific segment.",
                file.name
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use webui_protocol::{ComponentData, ComponentStyleClosure, FragmentList};

    #[test]
    fn retain_entry_protocol_prunes_style_closure_roots_and_resources() {
        let mut protocol = WebUIProtocol::default();
        for name in ["index.html", "kept-card", "removed-card"] {
            protocol
                .fragments
                .insert(name.to_string(), FragmentList::default());
        }
        for name in ["kept-card", "removed-card"] {
            protocol
                .components
                .insert(name.to_string(), ComponentData::default());
        }
        protocol.style_closures.insert(
            "index.html".to_string(),
            ComponentStyleClosure {
                component_tags: vec!["kept-card".to_string(), "removed-card".to_string()],
                style_chunks: Vec::new(),
            },
        );
        protocol.style_closures.insert(
            "removed-card".to_string(),
            ComponentStyleClosure {
                component_tags: vec!["removed-card".to_string()],
                style_chunks: Vec::new(),
            },
        );
        let graph = ComponentAssetGraph {
            files: vec![ComponentAssetFile {
                name: "root.js".to_string(),
                content: String::new(),
            }],
            metafile: None,
            entry_fragments: vec!["index.html".to_string(), "kept-card".to_string()],
            entry_components: vec!["kept-card".to_string()],
        };

        graph.retain_entry_protocol(&mut protocol);

        assert_eq!(
            protocol.style_closures["index.html"].component_tags,
            ["kept-card"]
        );
        assert!(!protocol.style_closures.contains_key("removed-card"));
    }

    #[test]
    fn component_asset_rejects_missing_style_closure_metadata() {
        let mut protocol = WebUIProtocol::default();
        protocol
            .fragments
            .insert("index.html".to_string(), FragmentList::default());
        protocol.fragments.insert(
            "legacy-card".to_string(),
            FragmentList {
                fragments: vec![webui_protocol::WebUIFragment::raw("<p>Legacy</p>")],
            },
        );
        protocol.components.insert(
            "legacy-card".to_string(),
            ComponentData {
                template_json: r#"{"h":"<p>Legacy</p>"}"#.to_string(),
                css: ".legacy{display:block}".to_string(),
                ..Default::default()
            },
        );

        let error = render_component_assets(
            &protocol,
            "index.html",
            &["legacy-card".to_string()],
            "[name].[ext]",
            false,
        )
        .expect_err("current component assets require style closure metadata");
        assert!(error
            .to_string()
            .contains("requires missing style closure metadata"));
    }

    #[test]
    fn component_asset_serializes_closures_only_for_owned_components() {
        let mut protocol = WebUIProtocol::default();
        protocol.set_css_strategy(webui_protocol::CssStrategy::Style);
        protocol.fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![webui_protocol::WebUIFragment::component("entry-card")],
            },
        );
        protocol
            .fragments
            .insert("entry-card".to_string(), FragmentList::default());
        protocol.fragments.insert(
            "deferred-card".to_string(),
            FragmentList {
                fragments: vec![webui_protocol::WebUIFragment::component("entry-card")],
            },
        );
        for tag in ["entry-card", "deferred-card"] {
            protocol.components.insert(
                tag.to_string(),
                ComponentData {
                    template_json: r#"{"h":"<div></div>"}"#.to_string(),
                    css: format!(".{tag}{{display:block}}"),
                    ..Default::default()
                },
            );
        }
        protocol.style_closures.insert(
            "entry-card".to_string(),
            ComponentStyleClosure {
                component_tags: vec!["entry-card".to_string()],
                style_chunks: Vec::new(),
            },
        );
        protocol.style_closures.insert(
            "deferred-card".to_string(),
            ComponentStyleClosure {
                component_tags: vec!["deferred-card".to_string(), "entry-card".to_string()],
                style_chunks: Vec::new(),
            },
        );

        let graph = render_component_assets(
            &protocol,
            "index.html",
            &["deferred-card".to_string()],
            "[name].[ext]",
            false,
        )
        .expect("render component asset");
        let asset = graph.files.first().expect("deferred root asset");

        assert!(asset
            .content
            .contains(r#""closures":{"deferred-card":["deferred-card","entry-card"]}"#));
        assert!(!asset.content.contains(r#""entry-card":["entry-card"]"#));
    }
}
