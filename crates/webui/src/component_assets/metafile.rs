// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use serde::Serialize;
use std::collections::BTreeMap;
use webui_handler::lazy_packs::LazyPackFile;
use webui_protocol::WebUIProtocol;

use super::serialize::RenderedOutput;
use crate::WebUIError;

/// A build metafile: esbuild-compatible `inputs`/`outputs` for component
/// assets, plus the lazy-region pack report.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Metafile<'a> {
    inputs: &'a BTreeMap<String, MetafileInput>,
    outputs: &'a BTreeMap<String, MetafileOutput>,
    lazy_packs: Vec<LazyPackEntry<'a>>,
}

/// The component asset graph half of a build metafile.
#[derive(Default)]
pub(crate) struct AssetGraphMetafile {
    inputs: BTreeMap<String, MetafileInput>,
    outputs: BTreeMap<String, MetafileOutput>,
}

/// One lazy-region pack: its cost and the other files the region loads.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LazyPackEntry<'a> {
    path: String,
    region: &'a str,
    owner: &'a str,
    trigger: &'static str,
    bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    brotli_bytes: Option<usize>,
    hash: &'a str,
    scripts: Vec<&'a str>,
    // Always present: empty unless a Link build's pack registers stylesheets.
    stylesheets: &'a [String],
}

#[derive(Serialize)]
struct MetafileInput {
    bytes: usize,
    imports: Vec<MetafileInputImport>,
    format: &'static str,
}

#[derive(Serialize)]
struct MetafileInputImport {
    path: String,
    kind: &'static str,
    #[serde(skip_serializing_if = "is_false")]
    external: bool,
    original: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MetafileOutput {
    bytes: usize,
    inputs: BTreeMap<String, MetafileOutputInput>,
    imports: Vec<MetafileOutputImport>,
    exports: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    entry_point: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MetafileOutputInput {
    bytes_in_output: usize,
}

#[derive(Serialize)]
struct MetafileOutputImport {
    path: String,
    kind: &'static str,
    #[serde(skip_serializing_if = "is_false")]
    external: bool,
}

/// Collect the esbuild-compatible description of rendered component assets.
pub(super) fn collect_asset_graph(
    protocol: &WebUIProtocol,
    rendered: &[RenderedOutput],
) -> AssetGraphMetafile {
    let mut graph = AssetGraphMetafile::default();
    for output in rendered {
        add_output_inputs(protocol, output, &mut graph.inputs);
        graph
            .outputs
            .insert(output.name.clone(), metafile_output(output));
    }
    graph
}

/// Serialize a build metafile from an optional component asset graph and the
/// build's lazy-region packs, sorted by path.
pub(crate) fn render_metafile(
    graph: Option<&AssetGraphMetafile>,
    packs: &[LazyPackFile],
) -> Result<String, WebUIError> {
    let empty = AssetGraphMetafile::default();
    let graph = graph.unwrap_or(&empty);
    let mut lazy_packs: Vec<LazyPackEntry<'_>> = packs
        .iter()
        .map(|pack| LazyPackEntry {
            path: pack.path(),
            region: &pack.region,
            owner: &pack.owner,
            trigger: pack.trigger.as_str(),
            bytes: pack.bytes(),
            brotli_bytes: pack.brotli_bytes,
            hash: pack.hash(),
            scripts: pack
                .scripts
                .iter()
                .map(|script| script.specifier.as_str())
                .collect(),
            stylesheets: &pack.stylesheets,
        })
        .collect();
    lazy_packs.sort_unstable_by(|left, right| left.path.cmp(&right.path));
    serde_json::to_string_pretty(&Metafile {
        inputs: &graph.inputs,
        outputs: &graph.outputs,
        lazy_packs,
    })
    .map_err(|error| {
        WebUIError::Serialization(format!("Failed to serialize the build metafile: {error}"))
    })
}

fn add_output_inputs(
    protocol: &WebUIProtocol,
    output: &RenderedOutput,
    inputs: &mut BTreeMap<String, MetafileInput>,
) {
    for required in &output.required_components {
        if is_external(output, required) {
            continue;
        }
        let path = component_path(required);
        inputs.entry(path).or_insert_with(|| MetafileInput {
            bytes: component_bytes(protocol, required),
            imports: Vec::new(),
            format: "esm",
        });
    }
    if let Some(root) = &output.root {
        let root_path = component_path(root);
        let root_input = inputs.entry(root_path).or_insert_with(|| MetafileInput {
            bytes: component_bytes(protocol, root),
            imports: Vec::new(),
            format: "esm",
        });
        root_input.imports = output
            .required_components
            .iter()
            .filter(|component| *component != root)
            .map(|component| MetafileInputImport {
                path: component_path(component),
                kind: "import-statement",
                external: is_external(output, component),
                original: component.clone(),
            })
            .collect();
    }
}

fn is_external(output: &RenderedOutput, component: &str) -> bool {
    output
        .external_components
        .binary_search_by(|candidate| candidate.as_str().cmp(component))
        .is_ok()
}

fn metafile_output(output: &RenderedOutput) -> MetafileOutput {
    let inputs = output
        .components
        .iter()
        .map(|(component, bytes)| {
            (
                component_path(component),
                MetafileOutputInput {
                    bytes_in_output: *bytes,
                },
            )
        })
        .collect();
    let imports = output
        .imports
        .iter()
        .map(|path| MetafileOutputImport {
            path: path.clone(),
            kind: "import-statement",
            external: false,
        })
        .collect();
    MetafileOutput {
        bytes: output.bytes,
        inputs,
        imports,
        exports: vec!["default"],
        entry_point: output.root.as_deref().map(component_path),
    }
}

fn component_path(component: &str) -> String {
    let mut path = String::with_capacity(component.len() + 16);
    path.push_str("webui:component/");
    path.push_str(component);
    path
}

fn component_bytes(protocol: &WebUIProtocol, component: &str) -> usize {
    protocol.components.get(component).map_or(0, |data| {
        component.len()
            + data.template.len()
            + data.template_json.len()
            + data.template_functions.len()
            + data.css.len()
            + data.css_href.len()
    })
}

fn is_false(value: &bool) -> bool {
    !*value
}
