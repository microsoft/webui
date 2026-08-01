// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use rayon::prelude::*;
use webui_protocol::WebUIProtocol;

use super::graph::{AssetGraphPlan, ChunkPlan, RootPlan};
use super::payload::render_component_payloads;
use super::serialize::{render_asset, PendingAsset, RenderedAsset, RenderedOutput, ResolvedImport};
use super::ComponentAssetFile;
use crate::{AssetFileNameTemplate, WebUIError};

pub(super) struct RenderedGraph {
    pub files: Vec<ComponentAssetFile>,
    pub outputs: Vec<RenderedOutput>,
}

pub(super) fn render_component_asset_graph(
    protocol: &WebUIProtocol,
    plan: &AssetGraphPlan,
    file_name_template: &AssetFileNameTemplate,
    emit_metafile: bool,
) -> Result<RenderedGraph, WebUIError> {
    let payloads = render_component_payloads(protocol, plan)?;
    let chunks = if plan.chunks.is_empty() {
        Vec::new()
    } else {
        let chunk_results: Vec<Result<RenderedAsset, WebUIError>> = plan
            .chunks
            .par_iter()
            .map(|chunk| {
                render_asset(
                    &pending_chunk(chunk),
                    plan,
                    &payloads,
                    file_name_template,
                    emit_metafile,
                )
            })
            .collect();
        collect_rendered(chunk_results)?
    };

    let roots = if plan.roots.len() == 1 {
        vec![render_asset(
            &pending_root(&plan.roots[0], plan, &chunks),
            plan,
            &payloads,
            file_name_template,
            emit_metafile,
        )?]
    } else {
        let root_results: Vec<Result<RenderedAsset, WebUIError>> = plan
            .roots
            .par_iter()
            .map(|root| {
                render_asset(
                    &pending_root(root, plan, &chunks),
                    plan,
                    &payloads,
                    file_name_template,
                    emit_metafile,
                )
            })
            .collect();
        collect_rendered(root_results)?
    };

    let mut files = Vec::with_capacity(roots.len() + chunks.len());
    let mut outputs = if emit_metafile {
        Vec::with_capacity(files.capacity())
    } else {
        Vec::new()
    };
    for rendered in roots.into_iter().chain(chunks) {
        files.push(rendered.file);
        if let Some(output) = rendered.output {
            outputs.push(output);
        }
    }
    Ok(RenderedGraph { files, outputs })
}

fn pending_chunk(chunk: &ChunkPlan) -> PendingAsset {
    PendingAsset {
        logical_name: chunk.name.clone(),
        root: None,
        components: chunk.components.clone(),
        required_components: chunk.components.clone(),
        external_components: Vec::new(),
        imports: Vec::new(),
    }
}

fn pending_root(root: &RootPlan, plan: &AssetGraphPlan, chunks: &[RenderedAsset]) -> PendingAsset {
    let imports = root
        .chunks
        .iter()
        .map(|chunk_id| ResolvedImport {
            file_name: chunks[*chunk_id].file.name.clone(),
            components: plan.chunks[*chunk_id].components.clone(),
        })
        .collect();
    PendingAsset {
        logical_name: root.root.clone(),
        root: Some(root.root.clone()),
        components: root.components.clone(),
        required_components: root.required_components.clone(),
        external_components: root.external_components.clone(),
        imports,
    }
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
