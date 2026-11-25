// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::BTreeMap;

use crate::{
    diagnostic, Import, Module, ModuleGraph, ProjectionDiagnostic, MAX_MODULES, MAX_MODULE_BYTES,
    MAX_SOURCE_BYTES,
};

pub(super) fn package_name<'a>(graph: &'a ModuleGraph, edge: &'a Import) -> Option<&'a str> {
    edge.package_name.as_deref().or_else(|| {
        graph
            .modules
            .get(&edge.resolved)
            .and_then(|module| module.package_name.as_deref())
    })
}

fn consistent_imports(graph: &ModuleGraph, module: &Module) -> bool {
    let mut imports = BTreeMap::new();
    for edge in module.imports.iter().filter(|edge| !edge.dynamic) {
        let identity = (
            edge.resolved.as_str(),
            edge.external,
            package_name(graph, edge),
        );
        if imports
            .insert(edge.specifier.as_str(), identity)
            .is_some_and(|old| old != identity)
        {
            return false;
        }
        let target_package = graph
            .modules
            .get(&edge.resolved)
            .and_then(|module| module.package_name.as_deref());
        if edge
            .package_name
            .as_deref()
            .zip(target_package)
            .is_some_and(|(edge, target)| edge != target)
        {
            return false;
        }
    }
    true
}

pub(super) fn validate(graph: &ModuleGraph) -> Vec<ProjectionDiagnostic> {
    let mut diagnostics = Vec::new();
    let total = graph.modules.values().fold(0_usize, |total, module| {
        total.saturating_add(module.source.as_ref().map_or(0, String::len))
    });
    if graph.modules.len() > MAX_MODULES
        || total > MAX_SOURCE_BYTES
        || graph.modules.values().any(|module| {
            module
                .source
                .as_ref()
                .is_some_and(|source| source.len() > MAX_MODULE_BYTES)
        })
    {
        diagnostics.push(diagnostic("", "PROJ-C013", "The native adapter source graph must not exceed 65,535 modules, 16 MiB per module or 64 MiB total."));
        return diagnostics;
    }
    for entry in &graph.entries {
        if !graph.modules.contains_key(entry) {
            diagnostics.push(diagnostic(
                entry,
                "PROJ-C013",
                "Every graph entry must identify an available module.",
            ));
        }
    }
    for (id, module) in &graph.modules {
        if module.opaque && module.syntax.is_some() {
            diagnostics.push(diagnostic(
                id,
                "PROJ-C013",
                "Opaque inputs must not declare a JavaScript/TypeScript grammar.",
            ));
        }
        if !id.starts_with('\0') && !module.opaque && module.source.is_none() {
            diagnostics.push(diagnostic(
                id,
                "PROJ-C013",
                "Provide original source for every physical module.",
            ));
        }
        if !consistent_imports(graph, module) {
            diagnostics.push(diagnostic(id, "PROJ-C013", "Static imports of one specifier must agree on resolution, external status and authoritative package identity."));
        }
        for edge in &module.imports {
            if !edge.external && !graph.modules.contains_key(&edge.resolved) {
                diagnostics.push(diagnostic(
                    id,
                    "PROJ-C013",
                    "Every non-external resolved edge must identify an available module.",
                ));
            }
        }
    }
    diagnostics
}
