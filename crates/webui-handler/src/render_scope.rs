// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Borrowed ordinary scopes and durable continuation bindings.

use std::borrow::Cow;
use std::collections::HashMap;
#[cfg(test)]
use std::sync::Arc;

use serde_json::Value;

mod repeat;
mod saved;
pub(crate) use repeat::{RepeatFrame, RepeatScratch, RepeatSource};
pub(crate) use saved::{SavedScope, SavedSharedScope};

use crate::state_view::SharedValue;
use crate::{
    find_value_by_dotted_path_ref, BorrowedScope, LocalValueSources, LoopBinding,
    ResolvedRenderFragmentIndex, StateView, VisibleLoopScope, WebUIProcessContext,
};

pub(crate) type SharedBindings = HashMap<String, SharedValue>;
pub(crate) type SharedAlias = Option<(Box<str>, SharedValue)>;

#[derive(Default)]
pub(crate) struct RenderScopes<'protocol, 'state> {
    pub(crate) alias: Option<(&'protocol str, &'state Value)>,
    pub(crate) shared_alias: SharedAlias,
    pub(crate) locals: SharedBindings,
    pub(crate) attrs: SharedBindings,
    pub(crate) loops: SharedBindings,
    pub(crate) borrowed_saves: Vec<BorrowedScope<'protocol, 'state>>,
    pub(crate) alias_saves: Vec<(&'protocol str, &'state Value)>,
    pub(crate) repeats: Vec<RepeatFrame<'protocol, 'state>>,
    pub(crate) shared_saves: Vec<SavedSharedScope>,
}

#[derive(Clone, Copy)]
pub(crate) struct Sources<'ctx, 'protocol, 'state> {
    pub(crate) scopes: &'ctx RenderScopes<'protocol, 'state>,
    pub(crate) loops: &'ctx [LoopBinding<'protocol, 'state>],
    pub(crate) visible: VisibleLoopScope,
    pub(crate) locals: LocalValueSources<'ctx, 'protocol, 'state>,
    pub(crate) state: StateView<'state>,
}

#[inline(always)]
pub(crate) fn resolve<'ctx, 'state: 'ctx>(
    path: &str,
    sources: Sources<'ctx, '_, 'state>,
) -> Option<Cow<'ctx, Value>> {
    if sources.visible.start < sources.visible.end && sources.scopes.loops.is_empty() {
        let binding = &sources.loops[sources.visible.end - 1];
        let name = binding.name;
        if path == name {
            return Some(Cow::Borrowed(binding.value));
        }
        if path.as_bytes().get(name.len()) == Some(&b'.') && path.starts_with(name) {
            return find_value_by_dotted_path_ref(&path[name.len() + 1..], binding.value)
                .or_else(|| resolve_state_fallback(path, sources.state));
        }
    }
    resolve_scoped(path, sources)
}

#[inline(never)]
fn resolve_state_fallback<'a>(path: &str, state: StateView<'a>) -> Option<Cow<'a, Value>> {
    state.resolve(path)
}

// Keep the general scope search out of the tiny innermost-loop lookup.
// Inlining it there prevents the loop lookup itself from inlining at bindings.
#[inline(never)]
fn resolve_scoped<'ctx, 'state: 'ctx>(
    path: &str,
    sources: Sources<'ctx, '_, 'state>,
) -> Option<Cow<'ctx, Value>> {
    let (root, rest) = split(path);
    if let Some(value) = sources.scopes.loops.get(root) {
        return relative(value.get(), rest).or_else(|| sources.state.resolve(path));
    }
    if let Some(binding) = sources.loops[sources.visible.start..sources.visible.end]
        .iter()
        .rev()
        .find(|binding| binding.name == root)
    {
        return relative(binding.value, rest).or_else(|| sources.state.resolve(path));
    }
    if let Some((name, value)) = sources.scopes.alias.filter(|(name, _)| *name == root) {
        let _ = name;
        return relative(value, rest);
    }
    if let Some((_, value)) = sources
        .scopes
        .shared_alias
        .as_ref()
        .filter(|(name, _)| name.as_ref() == root)
    {
        return relative(value.get(), rest);
    }
    if let Some(value) = sources.scopes.locals.get(root) {
        return relative(value.get(), rest).or_else(|| sources.state.resolve(path));
    }
    if let Some(value) = sources
        .locals
        .borrowed
        .get(root)
        .or_else(|| sources.locals.owned.get(root))
    {
        return relative(value, rest).or_else(|| sources.state.resolve(path));
    }
    sources.state.resolve(path)
}

/// A borrow that outlives mutations of the context's scope maps.
pub(crate) fn borrowed<'state>(
    path: &str,
    sources: Sources<'_, '_, 'state>,
) -> Option<&'state Value> {
    let (root, rest) = split(path);
    if sources.scopes.loops.contains_key(root) {
        return None;
    }
    if let Some(binding) = sources.loops[sources.visible.start..sources.visible.end]
        .iter()
        .rev()
        .find(|binding| binding.name == root)
    {
        return match relative(binding.value, rest) {
            Some(Cow::Borrowed(value)) => Some(value),
            Some(Cow::Owned(_)) => None,
            None => crate::borrowed_state_value(path, sources.state),
        };
    }
    if let Some((_, value)) = sources.scopes.alias.filter(|(name, _)| *name == root) {
        return borrowed_relative(value, rest);
    }
    if sources
        .scopes
        .shared_alias
        .as_ref()
        .is_some_and(|(name, _)| name.as_ref() == root)
        || sources.scopes.locals.contains_key(root)
    {
        return None;
    }
    crate::resolve_state_backed_value(
        path,
        sources.loops,
        sources.visible,
        sources.locals,
        sources.state,
    )
}

pub(crate) fn capture(
    path: &str,
    context: &mut WebUIProcessContext<'_, '_, '_>,
) -> Option<SharedValue> {
    if !context.state.is_shared() {
        match resolve(path, value_sources!(context)) {
            Some(Cow::Owned(value)) => {
                return Some(SharedValue::with_provenance(
                    value,
                    context.render_fragments.provenance_policy(),
                ));
            }
            None => return None,
            Some(Cow::Borrowed(_)) => {}
        }
    }
    let (root, rest) = split(path);
    let index = &context.render_fragments;
    if let Some(value) = context.scopes.loops.get(root) {
        return project(value, rest, index)
            .or_else(|| capture_state(root, rest, context.state, index));
    }
    if context.scopes.alias.is_some_and(|(name, _)| name == root) {
        return None;
    }
    if let Some((_, value)) = context
        .scopes
        .shared_alias
        .as_ref()
        .filter(|(name, _)| name.as_ref() == root)
    {
        return project(value, rest, index);
    }
    if let Some(value) = context.scopes.locals.get(root) {
        return project(value, rest, index)
            .or_else(|| capture_state(root, rest, context.state, index));
    }
    // Owned component props become immutable only when a durable call needs them.
    // Moving the root keeps nested captures from copying the same subtree.
    if let Some(value) = context.local_vars.remove(root) {
        let value = SharedValue::with_provenance(value, index.provenance_policy());
        let selected = project(&value, rest, index);
        context.scopes.locals.insert(root.to_owned(), value);
        return selected.or_else(|| capture_state(root, rest, context.state, index));
    }
    capture_state(root, rest, context.state, index)
}

fn capture_state(
    root: &str,
    projection: Option<&str>,
    state: StateView<'_>,
    index: &ResolvedRenderFragmentIndex<'_>,
) -> Option<SharedValue> {
    let value = state.capture(root)?;
    match projection {
        Some(path) => value.project_with_path(path, || index.capture_path(path)),
        None => Some(value),
    }
}

fn project(
    value: &SharedValue,
    rest: Option<&str>,
    index: &ResolvedRenderFragmentIndex<'_>,
) -> Option<SharedValue> {
    match rest {
        Some(rest) => value.project_with_path(rest, || index.capture_path(rest)),
        None => Some(value.clone()),
    }
}

fn split(path: &str) -> (&str, Option<&str>) {
    match path.split_once('.') {
        Some((root, rest)) => (root, Some(rest)),
        None => (path, None),
    }
}

fn relative<'a>(value: &'a Value, rest: Option<&str>) -> Option<Cow<'a, Value>> {
    match rest {
        Some(rest) => find_value_by_dotted_path_ref(rest, value),
        None => Some(Cow::Borrowed(value)),
    }
}

fn borrowed_relative<'a>(value: &'a Value, rest: Option<&str>) -> Option<&'a Value> {
    match relative(value, rest)? {
        Cow::Borrowed(value) => Some(value),
        Cow::Owned(_) => None,
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use webui_protocol::{FragmentList, WebUIFragment, WebUIProtocol};
    use webui_test_utils::test_json;

    #[test]
    fn borrowed_prop_lookup_preserves_hidden_loops_and_alias_precedence() {
        let state = test_json!({
            "item": {"missing": "owner"},
            "itemExtra": {"name": "different-root"}
        });
        let item = test_json!({"name": "borrowed-prop"});
        let hidden = test_json!({"name": "hidden-loop"});
        let alias = test_json!({"name": "alias"});
        let loops = [LoopBinding {
            name: "item",
            value: &hidden,
        }];
        let mut scopes = RenderScopes::default();
        let owned = HashMap::from([("item".into(), test_json!({"name": "owned-prop"}))]);
        let mut borrowed = BorrowedScope::default();
        borrowed.insert("item", &item);
        let check = |scopes: &RenderScopes<'_, '_>, path: &str, expected: Option<&str>| {
            let sources = Sources {
                scopes,
                loops: &loops,
                visible: VisibleLoopScope::EMPTY,
                locals: LocalValueSources {
                    owned: &owned,
                    borrowed: &borrowed,
                },
                state: (&state).into(),
            };
            assert_eq!(
                resolve(path, sources).as_deref().and_then(Value::as_str),
                expected
            );
        };
        check(&scopes, "item.name", Some("borrowed-prop"));
        check(&scopes, "item.missing", Some("owner"));
        check(&scopes, "itemExtra.name", Some("different-root"));
        scopes.loops.insert(
            "item".into(),
            SharedValue::new(test_json!({"name": "shared-loop"})),
        );
        check(&scopes, "item.name", Some("shared-loop"));
        scopes.loops.clear();
        scopes.locals.insert(
            "item".into(),
            SharedValue::new(test_json!({"name": "shared-prop"})),
        );
        check(&scopes, "item.name", Some("shared-prop"));
        scopes.locals.clear();
        scopes.alias = Some(("item", &alias));
        check(&scopes, "item.name", Some("alias"));
        check(&scopes, "item.missing", None);
        scopes.alias = None;
        scopes.shared_alias = Some((
            "item".into(),
            SharedValue::new(test_json!({"name": "shared-alias"})),
        ));
        check(&scopes, "item.name", Some("shared-alias"));
        check(&scopes, "item.missing", None);
    }

    #[test]
    fn innermost_borrowed_lookup_preserves_prefix_fallback_and_shared_precedence() {
        let state = test_json!({
            "item": {"missing": "owner"},
            "itemExtra": {"name": "different-root"}
        });
        let item = test_json!({"name": "borrowed"});
        let loops = [LoopBinding {
            name: "item",
            value: &item,
        }];
        let mut scopes = RenderScopes::default();
        let owned = HashMap::new();
        let borrowed = BorrowedScope::default();
        let sources = Sources {
            scopes: &scopes,
            loops: &loops,
            visible: VisibleLoopScope { start: 0, end: 1 },
            locals: LocalValueSources {
                owned: &owned,
                borrowed: &borrowed,
            },
            state: (&state).into(),
        };
        for (path, expected) in [
            ("item.name", "borrowed"),
            ("item.missing", "owner"),
            ("itemExtra.name", "different-root"),
        ] {
            assert_eq!(
                resolve(path, sources).as_deref().and_then(Value::as_str),
                Some(expected)
            );
        }
        scopes.loops.insert(
            "item".into(),
            SharedValue::new(test_json!({"name": "shared"})),
        );
        let sources = Sources {
            scopes: &scopes,
            loops: &loops,
            visible: VisibleLoopScope { start: 0, end: 1 },
            locals: LocalValueSources {
                owned: &owned,
                borrowed: &borrowed,
            },
            state: (&state).into(),
        };
        assert_eq!(
            resolve("item.name", sources)
                .as_deref()
                .and_then(Value::as_str),
            Some("shared")
        );
    }

    #[test]
    fn durable_projection_paths_reuse_compiled_backing() {
        let protocol = crate::Protocol::new(WebUIProtocol::new(HashMap::from([
            (
                "entry".into(),
                FragmentList {
                    fragments: vec![WebUIFragment::render("leaf", "tree.children", "row")],
                    contains_boundary: false,
                },
            ),
            ("leaf".into(), FragmentList::default()),
        ])));
        let index = protocol.render_fragments().resolve(protocol.protocol());
        let first = index.capture_path("children");
        let second = index.capture_path("children");
        assert!(Arc::ptr_eq(&first, &second));
        let root = SharedValue::new(test_json!({"children": ["OLD"]}));
        let child = root.project_shared(second).unwrap();
        let crate::state_view::SourceOrigin::Path { path, .. } = child.origin().as_ref() else {
            panic!("expected a projected input");
        };
        assert!(Arc::ptr_eq(&first, path));
    }
}
