// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Finite owner-state projection and pruning for local fragment call graphs.

use super::*;
use std::collections::{BTreeSet, VecDeque};

struct DirectReads<'a> {
    roots: Vec<String>,
    calls: Vec<&'a CompiledRender>,
}

pub(super) fn collect_build_metadata(meta: &TemplateMeta) -> TemplateBuildMetadata {
    let root_index = meta.declaration_count;
    let mut direct = Vec::with_capacity(root_index + 1);
    for block in meta.blocks.iter().take(meta.declaration_count) {
        direct.push(collect_direct_reads(meta, block));
    }
    let mut root = collect_direct_reads(meta, &meta.root);
    add_event_roots(&mut root.roots, &meta.root_events, &[], None);
    direct.push(root);

    // Direct(E) is filtered only by the alias on D -> E. Transitive owner
    // reads are never filtered again: neither caller aliases nor caller loops
    // are visible after entering E. Each (declaration, root) is queued once.
    let mut owners = vec![BTreeSet::new(); direct.len()];
    let mut callers = vec![Vec::new(); direct.len()];
    let mut pending = VecDeque::new();
    for (caller, reads) in direct.iter().enumerate() {
        for call in &reads.calls {
            callers[call.block_index].push(caller);
            for root in &direct[call.block_index].roots {
                if root != &call.alias && owners[caller].insert(root.as_str()) {
                    pending.push_back((caller, root.as_str()));
                }
            }
        }
    }
    while let Some((callee, root)) = pending.pop_front() {
        for &caller in &callers[callee] {
            if owners[caller].insert(root) {
                pending.push_back((caller, root));
            }
        }
    }
    let mut roots = direct[root_index].roots.clone();
    for root in &owners[root_index] {
        add_root(&mut roots, root, &[], None);
    }
    roots.sort_unstable();
    TemplateBuildMetadata {
        roots,
        has_events: !meta.root_events.is_empty()
            || !meta.root.events.is_empty()
            || meta.blocks.iter().any(|block| !block.events.is_empty()),
    }
}

fn collect_direct_reads<'a>(
    meta: &'a TemplateMeta,
    section: &'a TemplateSectionMeta,
) -> DirectReads<'a> {
    let mut reads = DirectReads {
        roots: Vec::new(),
        calls: Vec::new(),
    };
    let mut scopes = Vec::<RootScope<'_>>::new();
    let mut stack = vec![RootVisit {
        block: section,
        scope: None,
    }];
    while let Some(visit) = stack.pop() {
        let block = visit.block;
        add_binding_roots(&mut reads.roots, block, &scopes, visit.scope);
        for render in &block.renders {
            add_root(&mut reads.roots, &render.scope, &scopes, visit.scope);
            reads.calls.push(render);
        }
        for (condition, index) in &block.conditionals {
            add_condition_roots(&mut reads.roots, condition, &scopes, visit.scope);
            stack.push(RootVisit {
                block: &meta.blocks[*index],
                scope: visit.scope,
            });
        }
        for repeat in &block.repeats {
            add_root(&mut reads.roots, &repeat.collection, &scopes, visit.scope);
            let scope = scopes.len();
            scopes.push(RootScope {
                name: &repeat.item_var,
                parent: visit.scope,
            });
            stack.push(RootVisit {
                block: &meta.blocks[repeat.block_index],
                scope: Some(scope),
            });
        }
    }
    reads
}

fn add_binding_roots(
    roots: &mut Vec<String>,
    block: &TemplateSectionMeta,
    scopes: &[RootScope<'_>],
    scope: Option<usize>,
) {
    for (_, parts, _) in &block.text_runs {
        add_part_roots(roots, parts, scopes, scope);
    }
    for binding in &block.attr_bindings {
        match binding {
            CompiledAttrBinding::Simple { value, .. }
            | CompiledAttrBinding::Complex { value, .. } => {
                add_root(roots, value, scopes, scope);
            }
            CompiledAttrBinding::Boolean { condition, .. } => {
                add_condition_roots(roots, condition, scopes, scope);
            }
            CompiledAttrBinding::Template { parts, .. } => {
                add_part_roots(roots, parts, scopes, scope);
            }
        }
    }
    add_event_roots(roots, &block.events, scopes, scope);
}

fn add_event_roots(
    roots: &mut Vec<String>,
    events: &[EventBinding],
    scopes: &[RootScope<'_>],
    scope: Option<usize>,
) {
    for (_, _, args) in events {
        for arg in args {
            if let EventArg::Path(path) = arg {
                add_root(roots, path, scopes, scope);
            }
        }
    }
}

pub(super) fn prune_unreachable(meta: &mut TemplateMeta) {
    let mut reachable = vec![false; meta.blocks.len()];
    let mut pending = Vec::new();
    push_targets(&meta.root, &mut pending);
    while let Some(index) = pending.pop() {
        if std::mem::replace(&mut reachable[index], true) {
            continue;
        }
        push_targets(&meta.blocks[index], &mut pending);
    }
    let mut remap = vec![usize::MAX; meta.blocks.len()];
    let mut next = 0;
    for (index, live) in reachable.iter().enumerate() {
        if *live {
            remap[index] = next;
            next += 1;
        }
    }
    meta.declaration_count = reachable
        .iter()
        .take(meta.declaration_count)
        .filter(|live| **live)
        .count();
    let mut index = 0;
    meta.blocks.retain(|_| {
        let live = reachable[index];
        index += 1;
        live
    });
    remap_targets(&mut meta.root, &remap);
    for block in &mut meta.blocks {
        remap_targets(block, &remap);
    }
}

fn push_targets(block: &TemplateSectionMeta, pending: &mut Vec<usize>) {
    pending.extend(block.conditionals.iter().map(|(_, index)| *index));
    pending.extend(block.repeats.iter().map(|repeat| repeat.block_index));
    pending.extend(block.renders.iter().map(|render| render.block_index));
}

fn remap_targets(block: &mut TemplateSectionMeta, remap: &[usize]) {
    for (_, index) in &mut block.conditionals {
        *index = remap[*index];
    }
    for repeat in &mut block.repeats {
        repeat.block_index = remap[repeat.block_index];
    }
    for render in &mut block.renders {
        render.block_index = remap[render.block_index];
    }
}
