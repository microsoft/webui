// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::HashSet;
use webui_protocol::WebUIProtocol;

use super::traversal::{has_template_payload, CollectedClosure, GraphIndex, TraversalScratch};
use crate::WebUIError;

pub(super) struct AssetGraphPlan<'a> {
    pub component_names: Vec<&'a str>,
    pub roots: Vec<RootPlan>,
    pub emitted_components: Vec<usize>,
    pub entry_fragments: Vec<String>,
    pub entry_components: Vec<String>,
}

pub(super) struct RootPlan {
    pub root: String,
    pub components: Vec<usize>,
    pub required_components: Vec<usize>,
    pub style_components: Vec<usize>,
    pub external_components: Vec<usize>,
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
    let mut root_plans = Vec::with_capacity(canonical_roots.len());
    for (root_id, root) in canonical_roots.into_iter().enumerate() {
        let required_components = root_closures[root_id].components.clone();
        let style_components = std::mem::take(&mut root_closures[root_id].component_order);
        let mut components = Vec::with_capacity(required_components.len());
        let mut external_components = Vec::new();
        for component in &required_components {
            if entry_mask[*component] {
                external_components.push(*component);
            } else {
                components.push(*component);
            }
        }
        root_plans.push(RootPlan {
            root,
            components,
            required_components,
            style_components,
            external_components,
        });
    }

    let mut emitted_components = Vec::new();
    for root in &root_plans {
        emitted_components.extend_from_slice(&root.components);
    }
    emitted_components.sort_unstable();
    emitted_components.dedup();

    let plan = finalize_plan(index, entry_closure, root_plans, emitted_components);
    validate_style_coverage(protocol, &plan)?;
    Ok(plan)
}

fn validate_style_coverage(
    protocol: &WebUIProtocol,
    plan: &AssetGraphPlan<'_>,
) -> Result<(), WebUIError> {
    for root in &plan.roots {
        for component in &root.components {
            let tag = plan.component_names[*component];
            let closure = protocol.style_closure(tag).ok_or_else(|| {
                invalid_style_coverage(tag, "requires missing style closure metadata")
            })?;
            for resource in closure {
                let covered = plan
                    .component_names
                    .binary_search(&resource.as_str())
                    .ok()
                    .is_some_and(|id| root.required_components.binary_search(&id).is_ok());
                if !covered || protocol.component_style_resource(resource).is_none() {
                    return Err(invalid_style_coverage(
                        tag,
                        &format!("references unavailable style resource <{resource}>"),
                    ));
                }
            }
        }
    }
    Ok(())
}

#[cold]
#[inline(never)]
fn invalid_style_coverage(tag: &str, detail: &str) -> WebUIError {
    WebUIError::InvalidBuildOptions(format!(
        "component asset <{tag}> {detail}. Rebuild the protocol's style closures before emitting component assets."
    ))
}

fn finalize_plan<'a>(
    index: GraphIndex<'a>,
    entry_closure: CollectedClosure,
    roots: Vec<RootPlan>,
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
        emitted_components,
        entry_fragments,
        entry_components,
    }
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
