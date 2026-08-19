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
use webui_protocol::{ComponentAssetStylePreload, WebUIProtocol};

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
    /// Compiler-resolved Link stylesheet hrefs for intent-time root preloading.
    pub style_preloads: Vec<ComponentAssetStylePreload>,
    entry_fragments: Vec<String>,
    entry_components: Vec<String>,
}

impl ComponentAssetGraph {
    pub(crate) fn retain_entry_protocol(&mut self, protocol: &mut WebUIProtocol) {
        if self.files.is_empty() {
            return;
        }
        protocol.component_asset_style_preloads = std::mem::take(&mut self.style_preloads);
        protocol
            .fragments
            .retain(|name, _| self.entry_fragments.binary_search(name).is_ok());
        protocol
            .components
            .retain(|name, _| self.entry_components.binary_search(name).is_ok());
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
            style_preloads: Vec::new(),
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
    let style_preloads = collect_component_asset_style_preloads(protocol, &plan);
    validate_unique_asset_file_names(&rendered.files)?;
    let metafile = if emit_metafile {
        Some(metafile::render_metafile(protocol, &rendered.outputs)?)
    } else {
        None
    };

    Ok(ComponentAssetGraph {
        files: rendered.files,
        metafile,
        style_preloads,
        entry_fragments: plan.entry_fragments,
        entry_components: plan.entry_components,
    })
}

fn collect_component_asset_style_preloads(
    protocol: &WebUIProtocol,
    plan: &graph::AssetGraphPlan<'_>,
) -> Vec<ComponentAssetStylePreload> {
    let mut preloads = Vec::with_capacity(plan.roots.len());
    for root in &plan.roots {
        let mut style_hrefs = Vec::new();
        let mut seen = HashSet::with_capacity(root.required_components.len());
        for component in &root.style_components {
            if root.external_components.binary_search(component).is_ok() {
                continue;
            }
            let Some(href) = protocol
                .components
                .get(plan.component_names[*component])
                .map(|component| component.css_href.as_str())
                .filter(|href| !href.is_empty())
            else {
                continue;
            };
            if seen.insert(href) {
                style_hrefs.push(href.to_string());
            }
        }
        if style_hrefs.is_empty() {
            continue;
        }
        preloads.push(ComponentAssetStylePreload {
            root: root.root.clone(),
            style_hrefs,
        });
    }
    preloads
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
