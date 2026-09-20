// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use serde::Serialize;
use std::collections::BTreeMap;
use webui_protocol::WebUIProtocol;

use super::serialize::RenderedOutput;
use crate::WebUIError;

#[derive(Serialize)]
struct Metafile {
    inputs: BTreeMap<String, MetafileInput>,
    outputs: BTreeMap<String, MetafileOutput>,
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
}

pub(super) fn render_metafile(
    protocol: &WebUIProtocol,
    rendered: &[RenderedOutput],
) -> Result<String, WebUIError> {
    let mut inputs = BTreeMap::new();
    let mut outputs = BTreeMap::new();
    for output in rendered {
        add_output_inputs(protocol, output, &mut inputs);
        outputs.insert(output.name.clone(), metafile_output(output));
    }
    serde_json::to_string_pretty(&Metafile { inputs, outputs }).map_err(|error| {
        WebUIError::Serialization(format!(
            "Failed to serialize component asset metafile: {error}"
        ))
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
                kind: if is_dynamic(output, component) {
                    "dynamic-import"
                } else {
                    "import-statement"
                },
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

fn is_dynamic(output: &RenderedOutput, component: &str) -> bool {
    output
        .dynamic_components
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
            kind: "dynamic-import",
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
