// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{
    condition_plan, path_root, CompiledAttrBinding, CompiledAttrPart, ConditionExpr,
    TemplateSectionMeta,
};

pub(super) fn single_root<'a>(meta: &TemplateSectionMeta, roots: &'a [String]) -> Option<&'a str> {
    let [root] = roots else {
        return None;
    };
    if !meta.conditionals.is_empty() || !meta.repeats.is_empty() {
        return None;
    }
    (meta
        .text_runs
        .iter()
        .all(|(_, parts, _)| parts_use_root(parts, root))
        && meta.attr_bindings.iter().all(|binding| match binding {
            CompiledAttrBinding::Simple { value, .. }
            | CompiledAttrBinding::Complex { value, .. } => path_root(value) == root,
            CompiledAttrBinding::Template { parts, .. } => parts_use_root(parts, root),
            CompiledAttrBinding::Boolean { condition, .. } => condition_uses_root(condition, root),
        }))
    .then_some(root.as_str())
}

fn parts_use_root(parts: &[CompiledAttrPart], root: &str) -> bool {
    let mut dynamic = false;
    for part in parts {
        if let CompiledAttrPart::Dynamic(path) = part {
            if path_root(path) != root {
                return false;
            }
            dynamic = true;
        }
    }
    dynamic
}

fn condition_uses_root(condition: &ConditionExpr, root: &str) -> bool {
    let mut dynamic = false;
    condition_plan::visit_paths(condition, |path| {
        dynamic = true;
        path_root(path) == root
    }) && dynamic
}
