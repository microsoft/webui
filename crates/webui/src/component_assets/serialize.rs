// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::graph::AssetGraphPlan;
use super::json::{push_json_string, push_u64};
use super::payload::{render_style_resource, RenderedComponent, RenderedStyleResource};
use super::ComponentAssetFile;
use crate::{AssetFileNameTemplate, WebUIError};
use webui_protocol::WebUIProtocol;

const ASSET_TYPE: &str = "webui-component-asset";
const ASSET_VERSION: u64 = 3;
const COMPONENT_ASSET_EXT: &str = "webui.js";

pub(super) struct RenderedOutput {
    pub name: String,
    pub bytes: usize,
    pub root: Option<String>,
    pub components: Vec<(String, usize)>,
    pub required_components: Vec<String>,
    pub external_components: Vec<String>,
    pub dynamic_components: Vec<String>,
    pub imports: Vec<String>,
}

pub(super) struct PendingAsset {
    pub logical_name: String,
    pub root: Option<String>,
    pub components: Vec<usize>,
    pub required_components: Vec<usize>,
    pub external_components: Vec<usize>,
    pub imports: Vec<ResolvedImport>,
}

pub(super) struct ResolvedImport {
    pub file_name: String,
    pub components: Vec<usize>,
}

pub(super) struct RenderedAsset {
    pub file: ComponentAssetFile,
    pub output: Option<RenderedOutput>,
}

pub(super) struct AssetRenderOptions<'a> {
    pub file_name_template: &'a AssetFileNameTemplate,
    pub emit_metafile: bool,
    pub protocol: &'a WebUIProtocol,
}

struct StyleResourceWriter<'a, 'data> {
    out: &'a mut String,
    plan: &'a AssetGraphPlan<'data>,
    payloads: &'a [Option<RenderedComponent<'data>>],
    protocol: &'data WebUIProtocol,
    attribution: &'a mut [usize],
}

pub(super) fn render_asset(
    pending: &PendingAsset,
    plan: &AssetGraphPlan,
    payloads: &[Option<RenderedComponent<'_>>],
    options: &AssetRenderOptions<'_>,
) -> Result<RenderedAsset, WebUIError> {
    let mut js = String::with_capacity(estimate_asset_size(pending, plan, payloads));
    let mut attribution = if options.emit_metafile {
        vec![0usize; pending.components.len()]
    } else {
        Vec::new()
    };
    js.push_str("const asset={\"type\":\"");
    js.push_str(ASSET_TYPE);
    js.push_str("\",\"version\":");
    push_u64(&mut js, ASSET_VERSION);
    js.push_str(",\"kind\":\"");
    js.push_str(if pending.root.is_some() {
        "root"
    } else {
        "chunk"
    });
    js.push('"');
    if let Some(root) = &pending.root {
        js.push_str(",\"root\":");
        push_json_string(&mut js, root, "component asset root")?;
    }
    js.push_str(",\"components\":[");
    push_component_tags(&mut js, &pending.components, plan, &mut attribution)?;
    js.push_str("],\"requiredComponents\":[");
    push_component_id_array(&mut js, &pending.required_components, plan)?;
    js.push_str("],\"externalComponents\":[");
    push_component_id_array(&mut js, &pending.external_components, plan)?;
    js.push_str("],\"imports\":[");
    push_imports(&mut js, &pending.imports, plan)?;
    js.push_str("],\"componentStyles\":{\"version\":1,\"strategy\":\"");
    js.push_str(options.protocol.css_strategy().wire_name());
    js.push_str("\",\"resources\":{");
    {
        let mut writer = StyleResourceWriter {
            out: &mut js,
            plan,
            payloads,
            protocol: options.protocol,
            attribution: &mut attribution,
        };
        let written_resources =
            push_style_resources(&mut writer, &pending.components, 0, true, true)?;
        push_style_resources(
            &mut writer,
            &pending.external_components,
            written_resources,
            false,
            false,
        )?;
    }
    js.push_str("},\"closures\":{");
    push_style_closures(&mut js, pending, plan, options.protocol)?;
    js.push_str("}}");
    js.push_str(",\"templates\":{");
    push_templates(
        &mut js,
        &pending.components,
        plan,
        payloads,
        &mut attribution,
    )?;
    js.push('}');
    if pending.components.iter().any(|id| {
        payloads
            .get(*id)
            .and_then(Option::as_ref)
            .is_some_and(|payload| payload.functions.is_some())
    }) {
        js.push_str(",\"templateFunctions\":{");
        push_functions(
            &mut js,
            &pending.components,
            plan,
            payloads,
            &mut attribution,
        )?;
        js.push('}');
    }
    js.push_str("};\nexport default asset;\n");

    let name = options.file_name_template.resolve(
        &pending.logical_name,
        COMPONENT_ASSET_EXT,
        js.as_bytes(),
    );
    let output = options.emit_metafile.then(|| {
        let mut dynamic_components: Vec<String> = pending
            .imports
            .iter()
            .flat_map(|import| import.components.iter())
            .map(|component| plan.component_names[*component].to_string())
            .collect();
        dynamic_components.sort_unstable();
        dynamic_components.dedup();
        RenderedOutput {
            bytes: js.len(),
            root: pending.root.clone(),
            components: pending
                .components
                .iter()
                .enumerate()
                .map(|(index, id)| (plan.component_names[*id].to_string(), attribution[index]))
                .collect(),
            required_components: component_names(&pending.required_components, plan),
            external_components: component_names(&pending.external_components, plan),
            dynamic_components,
            imports: pending
                .imports
                .iter()
                .map(|import| import.file_name.clone())
                .collect(),
            name: name.clone(),
        }
    });
    Ok(RenderedAsset {
        file: ComponentAssetFile { name, content: js },
        output,
    })
}

fn estimate_asset_size(
    pending: &PendingAsset,
    plan: &AssetGraphPlan,
    payloads: &[Option<RenderedComponent<'_>>],
) -> usize {
    let mut size = 256 + pending.logical_name.len();
    for component in &pending.components {
        size += plan.component_names[*component].len() * 3 + 16;
        if let Some(payload) = payloads.get(*component).and_then(Option::as_ref) {
            size += payload.template.len();
            size += payload.resource.as_ref().map_or(0, |resource| {
                let bytes = match resource {
                    RenderedStyleResource::Link(value)
                    | RenderedStyleResource::Style(value)
                    | RenderedStyleResource::Module(value) => value.len(),
                };
                bytes + 64
            });
            size += payload.functions.map_or(0, str::len);
        }
    }
    for import in &pending.imports {
        size += import.file_name.len() * 2 + 96;
    }
    size
}

fn push_component_tags(
    out: &mut String,
    components: &[usize],
    plan: &AssetGraphPlan,
    attribution: &mut [usize],
) -> Result<(), WebUIError> {
    for (index, component) in components.iter().copied().enumerate() {
        let start = out.len();
        if index > 0 {
            out.push(',');
        }
        push_json_string(out, plan.component_names[component], "component tag")?;
        add_attribution(attribution, index, out.len() - start);
    }
    Ok(())
}

fn push_component_id_array(
    out: &mut String,
    components: &[usize],
    plan: &AssetGraphPlan,
) -> Result<(), WebUIError> {
    for (index, component) in components.iter().copied().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_json_string(out, plan.component_names[component], "component tag")?;
    }
    Ok(())
}

fn push_imports(
    out: &mut String,
    imports: &[ResolvedImport],
    plan: &AssetGraphPlan,
) -> Result<(), WebUIError> {
    for (index, import) in imports.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"components\":[");
        push_component_id_array(out, &import.components, plan)?;
        out.push_str("],\"href\":new URL(");
        let mut relative = String::with_capacity(import.file_name.len() + 2);
        relative.push_str("./");
        relative.push_str(&import.file_name);
        push_json_string(out, &relative, "component asset import")?;
        out.push_str(",import.meta.url).href,\"load\":()=>import(");
        push_json_string(out, &relative, "component asset import")?;
        out.push_str(")}");
    }
    Ok(())
}

fn push_style_resources(
    writer: &mut StyleResourceWriter<'_, '_>,
    components: &[usize],
    mut written: usize,
    attribute_bytes: bool,
    resolve_relative_href: bool,
) -> Result<usize, WebUIError> {
    for (index, component) in components.iter().copied().enumerate() {
        let resource = match writer.payloads.get(component).and_then(Option::as_ref) {
            Some(payload) => payload.resource,
            None => render_style_resource(writer.protocol, writer.plan.component_names[component])?,
        };
        let Some(resource) = resource else {
            continue;
        };
        let start = writer.out.len();
        if written > 0 {
            writer.out.push(',');
        }
        let tag = writer.plan.component_names[component];
        push_json_string(writer.out, tag, "component style resource ID")?;
        match &resource {
            RenderedStyleResource::Link(href) => {
                writer.out.push_str(":{\"kind\":\"link\",\"href\":");
                if resolve_relative_href && is_relative_href(href) {
                    writer.out.push_str("new URL(");
                    push_json_string(writer.out, href, "component style href")?;
                    writer.out.push_str(",import.meta.url).href");
                } else {
                    push_json_string(writer.out, href, "component style href")?;
                }
                writer.out.push('}');
            }
            RenderedStyleResource::Style(css) => {
                writer.out.push_str(":{\"kind\":\"style\",\"css\":");
                push_json_string(writer.out, css, "component style CSS")?;
                writer.out.push('}');
            }
            RenderedStyleResource::Module(css) => {
                writer.out.push_str(":{\"kind\":\"module\",\"specifier\":");
                push_json_string(writer.out, tag, "component style module specifier")?;
                writer.out.push_str(",\"css\":");
                push_json_string(writer.out, css, "component style module CSS")?;
                writer.out.push('}');
            }
        }
        if attribute_bytes {
            add_attribution(writer.attribution, index, writer.out.len() - start);
        }
        written += 1;
    }
    Ok(written)
}

fn push_style_closures(
    out: &mut String,
    pending: &PendingAsset,
    plan: &AssetGraphPlan,
    protocol: &WebUIProtocol,
) -> Result<(), WebUIError> {
    for (index, component) in pending.components.iter().copied().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let tag = plan.component_names[component];
        push_json_string(out, tag, "component style closure root")?;
        out.push_str(":[");
        let closure = protocol.style_closures.get(tag).ok_or_else(|| {
            WebUIError::InvalidBuildOptions(format!(
                "component asset graph requires missing style closure metadata for <{tag}>"
            ))
        })?;
        for (closure_index, resource) in closure.component_tags.iter().enumerate() {
            if closure_index > 0 {
                out.push(',');
            }
            push_json_string(out, resource, "component style closure resource")?;
        }
        out.push(']');
    }
    Ok(())
}

fn is_relative_href(href: &str) -> bool {
    !href.starts_with('/')
        && !href.starts_with('#')
        && !href.starts_with("//")
        && !href.contains(':')
}

fn push_templates(
    out: &mut String,
    components: &[usize],
    plan: &AssetGraphPlan,
    payloads: &[Option<RenderedComponent<'_>>],
    attribution: &mut [usize],
) -> Result<(), WebUIError> {
    for (index, component) in components.iter().copied().enumerate() {
        let payload = payload(payloads, component)?;
        let start = out.len();
        if index > 0 {
            out.push(',');
        }
        push_json_string(out, plan.component_names[component], "component tag")?;
        out.push(':');
        out.push_str(&payload.template);
        add_attribution(attribution, index, out.len() - start);
    }
    Ok(())
}

fn push_functions(
    out: &mut String,
    components: &[usize],
    plan: &AssetGraphPlan,
    payloads: &[Option<RenderedComponent<'_>>],
    attribution: &mut [usize],
) -> Result<(), WebUIError> {
    let mut written = 0usize;
    for (index, component) in components.iter().copied().enumerate() {
        let payload = payload(payloads, component)?;
        let Some(functions) = &payload.functions else {
            continue;
        };
        let start = out.len();
        if written > 0 {
            out.push(',');
        }
        push_json_string(out, plan.component_names[component], "component tag")?;
        out.push(':');
        out.push_str(functions);
        add_attribution(attribution, index, out.len() - start);
        written += 1;
    }
    Ok(())
}

fn payload<'p, 'd>(
    payloads: &'p [Option<RenderedComponent<'d>>],
    component: usize,
) -> Result<&'p RenderedComponent<'d>, WebUIError> {
    payloads
        .get(component)
        .and_then(Option::as_ref)
        .ok_or_else(|| {
            WebUIError::InvalidBuildOptions(
                "component asset graph references a missing rendered payload".to_string(),
            )
        })
}

#[inline]
fn add_attribution(attribution: &mut [usize], index: usize, bytes: usize) {
    if let Some(value) = attribution.get_mut(index) {
        *value += bytes;
    }
}

fn component_names(components: &[usize], plan: &AssetGraphPlan) -> Vec<String> {
    components
        .iter()
        .map(|component| plan.component_names[*component].to_string())
        .collect()
}
