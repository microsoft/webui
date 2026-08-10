// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::cmp::Ordering;
use std::collections::HashSet;
use webui_protocol::WebUIProtocol;

use super::traversal::{has_template_payload, CollectedClosure, GraphIndex, TraversalScratch};
use crate::chunking::ConsumerMatrix;
use crate::WebUIError;

pub(super) struct AssetGraphPlan<'a> {
    pub component_names: Vec<&'a str>,
    pub roots: Vec<RootPlan>,
    pub chunks: Vec<ChunkPlan>,
    pub emitted_components: Vec<usize>,
    pub entry_fragments: Vec<String>,
    pub entry_components: Vec<String>,
}

pub(super) struct RootPlan {
    pub root: String,
    pub components: Vec<usize>,
    pub required_components: Vec<usize>,
    pub external_components: Vec<usize>,
    pub chunks: Vec<usize>,
}

pub(super) struct ChunkPlan {
    pub name: String,
    pub components: Vec<usize>,
    pub consumers: Vec<usize>,
}

pub(super) fn plan_component_assets<'a>(
    protocol: &'a WebUIProtocol,
    entry: &str,
    roots: &[String],
) -> Result<AssetGraphPlan<'a>, WebUIError> {
    let index = GraphIndex::new(protocol);
    let roots = validate_roots(protocol, roots)?;
    let mut canonical_roots = roots;
    canonical_roots.sort_unstable();

    let mut scratch = TraversalScratch::new(index.fragment_names.len());
    let entry_closure = scratch.collect(protocol, &index, entry)?;
    let mut entry_mask = vec![false; index.component_names.len()];
    for component in &entry_closure.components {
        entry_mask[*component] = true;
    }

    let mut root_closures = Vec::with_capacity(canonical_roots.len());
    for root in &canonical_roots {
        root_closures.push(scratch.collect(protocol, &index, root)?);
    }
    if canonical_roots.len() == 1 {
        let required_components = std::mem::take(&mut root_closures[0].components);
        let mut components = Vec::with_capacity(required_components.len());
        let mut external_components = Vec::new();
        for component in &required_components {
            if entry_mask[*component] {
                external_components.push(*component);
            } else {
                components.push(*component);
            }
        }
        let emitted_components = components.clone();
        return Ok(finalize_plan(
            index,
            entry_closure,
            vec![RootPlan {
                root: canonical_roots.swap_remove(0),
                components,
                required_components,
                external_components,
                chunks: Vec::new(),
            }],
            Vec::new(),
            emitted_components,
        ));
    }

    let mut consumers = ConsumerMatrix::new(index.component_names.len(), canonical_roots.len())
        .ok_or_else(component_graph_too_large)?;
    for (root_id, closure) in root_closures.iter().enumerate() {
        for component in &closure.components {
            if !entry_mask[*component] {
                consumers.insert(*component, root_id);
            }
        }
    }

    let mut local_components = vec![Vec::new(); canonical_roots.len()];
    let mut shared_components = Vec::new();
    for (component, is_entry_component) in entry_mask.iter().copied().enumerate() {
        if is_entry_component {
            continue;
        }
        match consumers.count(component) {
            0 => {}
            1 => local_components[consumers.single(component)].push(component),
            _ => shared_components.push(component),
        }
    }

    // Sorting by consumer set makes every equal set adjacent, so run grouping
    // merges all of them. The name tiebreak keeps chunk membership stable.
    shared_components.sort_unstable_by(|left, right| {
        let rows = consumers.row(*left).cmp(consumers.row(*right));
        if rows == Ordering::Equal {
            index.component_names[*left].cmp(index.component_names[*right])
        } else {
            rows
        }
    });

    let mut chunks =
        group_shared_components(&shared_components, &consumers, &index.component_names);
    chunks.sort_unstable_by(|left, right| left.name.cmp(&right.name));

    let mut root_chunk_ids = vec![Vec::new(); canonical_roots.len()];
    for (chunk_id, chunk) in chunks.iter().enumerate() {
        for consumer in &chunk.consumers {
            root_chunk_ids[*consumer].push(chunk_id);
        }
    }

    let mut root_plans = Vec::with_capacity(canonical_roots.len());
    for (root_id, root) in canonical_roots.into_iter().enumerate() {
        let required_components = root_closures[root_id].components.clone();
        let external_components = required_components
            .iter()
            .copied()
            .filter(|component| entry_mask[*component])
            .collect();
        root_plans.push(RootPlan {
            root,
            components: std::mem::take(&mut local_components[root_id]),
            required_components,
            external_components,
            chunks: std::mem::take(&mut root_chunk_ids[root_id]),
        });
    }

    let mut emitted_components = Vec::new();
    for root in &root_plans {
        emitted_components.extend_from_slice(&root.components);
    }
    for chunk in &chunks {
        emitted_components.extend_from_slice(&chunk.components);
    }
    emitted_components.sort_unstable();
    emitted_components.dedup();

    Ok(finalize_plan(
        index,
        entry_closure,
        root_plans,
        chunks,
        emitted_components,
    ))
}

fn finalize_plan<'a>(
    index: GraphIndex<'a>,
    entry_closure: CollectedClosure,
    roots: Vec<RootPlan>,
    chunks: Vec<ChunkPlan>,
    emitted_components: Vec<usize>,
) -> AssetGraphPlan<'a> {
    let mut entry_fragments: Vec<String> = entry_closure
        .fragments
        .into_iter()
        .map(|id| index.fragment_names[id].to_string())
        .collect();
    entry_fragments.sort_unstable();
    let mut entry_components: Vec<String> = entry_closure
        .components
        .into_iter()
        .map(|id| index.component_names[id].to_string())
        .collect();
    entry_components.sort_unstable();

    AssetGraphPlan {
        component_names: index.component_names,
        roots,
        chunks,
        emitted_components,
        entry_fragments,
        entry_components,
    }
}

fn group_shared_components(
    components: &[usize],
    consumers: &ConsumerMatrix,
    component_names: &[&str],
) -> Vec<ChunkPlan> {
    crate::chunking::group_runs(components, consumers)
        .into_iter()
        .map(|run| {
            let mut chunk_components = components[run.clone()].to_vec();
            chunk_components.sort_unstable_by_key(|component| component_names[*component]);
            let name = format!("chunk-{}", component_names[chunk_components[0]]);
            ChunkPlan {
                name,
                components: chunk_components,
                consumers: consumers.expand(components[run.start]),
            }
        })
        .collect()
}

fn validate_roots(protocol: &WebUIProtocol, roots: &[String]) -> Result<Vec<String>, WebUIError> {
    let mut seen = HashSet::with_capacity(roots.len());
    let mut normalized = Vec::with_capacity(roots.len());
    for raw in roots {
        let tag = raw.trim();
        validate_root(protocol, tag, &mut seen)?;
        normalized.push(tag.to_string());
    }
    Ok(normalized)
}

fn validate_root(
    protocol: &WebUIProtocol,
    tag: &str,
    seen: &mut HashSet<String>,
) -> Result<(), WebUIError> {
    if tag.is_empty() {
        return Err(WebUIError::InvalidBuildOptions(
            "--emit-component-assets contains an empty component tag".to_string(),
        ));
    }
    if !is_component_tag_name(tag) {
        return Err(WebUIError::InvalidBuildOptions(format!(
            "--emit-component-assets component '{tag}' must be a lowercase kebab-case custom element tag"
        )));
    }
    if !seen.insert(tag.to_string()) {
        return Err(WebUIError::InvalidBuildOptions(format!(
            "--emit-component-assets contains duplicate component <{tag}>"
        )));
    }
    if !protocol.fragments.contains_key(tag) {
        return Err(WebUIError::InvalidBuildOptions(format!(
            "--emit-component-assets requested unknown component <{tag}>. Add a discovered {tag}.html component or remove it from the allowlist."
        )));
    }
    if !protocol
        .components
        .get(tag)
        .is_some_and(has_template_payload)
    {
        return Err(WebUIError::InvalidBuildOptions(format!(
            "--emit-component-assets requested <{tag}>, but it has no compiled template metadata. Build with a plugin that emits component templates and ensure the component has a template."
        )));
    }
    Ok(())
}

fn is_component_tag_name(tag: &str) -> bool {
    let bytes = tag.as_bytes();
    !bytes.is_empty()
        && bytes.contains(&b'-')
        && bytes[0].is_ascii_lowercase()
        && bytes[bytes.len() - 1].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

#[cold]
#[inline(never)]
fn component_graph_too_large() -> WebUIError {
    WebUIError::InvalidBuildOptions(
        "component asset graph is too large to index safely".to_string(),
    )
}
