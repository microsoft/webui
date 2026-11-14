// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{
    condition_expr, is_condition_literal, path_root, CompiledAttrBinding, CompiledAttrPart,
    ConditionExpr, TemplateSectionMeta,
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
    let mut stack = Vec::with_capacity(2);
    stack.push(condition);
    let mut dynamic = false;
    while let Some(condition) = stack.pop() {
        match &condition.expr {
            Some(condition_expr::Expr::Identifier(id)) => {
                if id.value == "true" || id.value == "false" {
                    continue;
                }
                if path_root(&id.value) != root {
                    return false;
                }
                dynamic = true;
            }
            Some(condition_expr::Expr::Predicate(predicate)) => {
                if path_root(&predicate.left) != root
                    || (!is_condition_literal(&predicate.right)
                        && path_root(&predicate.right) != root)
                {
                    return false;
                }
                dynamic = true;
            }
            Some(condition_expr::Expr::Not(value)) => stack.extend(value.condition.as_deref()),
            Some(condition_expr::Expr::Compound(value)) => {
                stack.extend(value.left.as_deref());
                stack.extend(value.right.as_deref());
            }
            None => {}
        }
    }
    dynamic
}
