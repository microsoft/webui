// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use rayon::prelude::*;
use std::fmt::Write;
use webui_protocol::WebUIProtocol;

use super::graph::{AssetGraphPlan, RootPlan};
use super::payload::render_component_payloads;
use super::serialize::{
    render_asset, AssetRenderOptions, PendingAsset, RenderedAsset, RenderedOutput, ResolvedImport,
};
use super::ComponentAssetFile;
use crate::WebUIError;

const PARALLEL_RENDER_THRESHOLD: usize = 128;

pub(super) struct RenderedGraph {
    pub files: Vec<ComponentAssetFile>,
    pub outputs: Vec<RenderedOutput>,
}

pub(super) fn render_component_asset_graph(
    protocol: &WebUIProtocol,
    plan: &AssetGraphPlan,
    emit_metafile: bool,
) -> Result<RenderedGraph, WebUIError> {
    let payloads = render_component_payloads(protocol, plan)?;
    let render_options = AssetRenderOptions {
        emit_metafile,
        protocol,
    };
    let mut payloads_rendered = if plan.emitted_components.len() >= PARALLEL_RENDER_THRESHOLD {
        let payload_results: Vec<Result<RenderedAsset, WebUIError>> = plan
            .emitted_components
            .par_iter()
            .map(|component| {
                render_asset(
                    &pending_component(*component, plan),
                    plan,
                    &payloads,
                    &render_options,
                )
            })
            .collect();
        collect_rendered(payload_results)?
    } else {
        let mut rendered = Vec::with_capacity(plan.emitted_components.len());
        for component in &plan.emitted_components {
            rendered.push(render_asset(
                &pending_component(*component, plan),
                plan,
                &payloads,
                &render_options,
            )?);
        }
        rendered
    };
    content_address_payloads(&mut payloads_rendered)?;

    let roots = if plan.roots.len() >= PARALLEL_RENDER_THRESHOLD {
        let root_results: Vec<Result<RenderedAsset, WebUIError>> = plan
            .roots
            .par_iter()
            .map(|root| {
                render_asset(
                    &pending_root(root, plan, &payloads_rendered)?,
                    plan,
                    &payloads,
                    &render_options,
                )
            })
            .collect();
        collect_rendered(root_results)?
    } else {
        let mut rendered = Vec::with_capacity(plan.roots.len());
        for root in &plan.roots {
            rendered.push(render_asset(
                &pending_root(root, plan, &payloads_rendered)?,
                plan,
                &payloads,
                &render_options,
            )?);
        }
        rendered
    };

    let mut files = Vec::with_capacity(roots.len() + payloads_rendered.len());
    let mut outputs = if emit_metafile {
        Vec::with_capacity(files.capacity())
    } else {
        Vec::new()
    };
    for rendered in roots.into_iter().chain(payloads_rendered) {
        files.push(rendered.file);
        if let Some(output) = rendered.output {
            outputs.push(output);
        }
    }
    Ok(RenderedGraph { files, outputs })
}

fn content_address_payloads(payloads: &mut [RenderedAsset]) -> Result<(), WebUIError> {
    for payload in payloads {
        let Some(stem) = payload.file.name.strip_suffix(".webui.js") else {
            return Err(WebUIError::InvalidBuildOptions(
                "generated component payload has an invalid filename".to_string(),
            ));
        };
        let content = payload.file.content.as_bytes();
        let content_len = u32::try_from(content.len()).map_err(|_| {
            WebUIError::InvalidBuildOptions(
                "generated component payload exceeds the supported size".to_string(),
            )
        })?;
        let content_crc = crc32fast::hash(content);
        let mut name = String::with_capacity(stem.len() + 26);
        name.push_str(stem);
        name.push('.');
        write!(&mut name, "{content_len:08x}{content_crc:08x}").map_err(|_| {
            WebUIError::InvalidBuildOptions(
                "failed to encode component payload content hash".to_string(),
            )
        })?;
        name.push_str(".webui.js");
        payload.file.name = name.clone();
        if let Some(output) = &mut payload.output {
            output.name = name;
        }
    }
    Ok(())
}

fn pending_component(component: usize, plan: &AssetGraphPlan) -> PendingAsset {
    let tag = plan.component_names[component];
    let mut logical_name = String::with_capacity(tag.len() + 11);
    logical_name.push_str("components/");
    logical_name.push_str(tag);
    PendingAsset {
        logical_name,
        root: None,
        components: vec![component],
        required_components: vec![component],
        external_components: Vec::new(),
        imports: Vec::new(),
    }
}

fn pending_root(
    root: &RootPlan,
    plan: &AssetGraphPlan,
    payloads: &[RenderedAsset],
) -> Result<PendingAsset, WebUIError> {
    let imports = root
        .components
        .iter()
        .map(|component| {
            let payload_index = plan
                .emitted_components
                .binary_search(component)
                .map_err(|_| {
                    WebUIError::InvalidBuildOptions(
                        "component asset root references a missing generated component module"
                            .to_string(),
                    )
                })?;
            Ok(ResolvedImport {
                file_name: payloads[payload_index].file.name.clone(),
            })
        })
        .collect::<Result<Vec<_>, WebUIError>>()?;
    Ok(PendingAsset {
        logical_name: root.root.clone(),
        root: Some(root.root.clone()),
        components: Vec::new(),
        required_components: root.required_components.clone(),
        external_components: root.external_components.clone(),
        imports,
    })
}

fn collect_rendered(
    results: Vec<Result<RenderedAsset, WebUIError>>,
) -> Result<Vec<RenderedAsset>, WebUIError> {
    let mut rendered = Vec::with_capacity(results.len());
    for result in results {
        rendered.push(result?);
    }
    Ok(rendered)
}
