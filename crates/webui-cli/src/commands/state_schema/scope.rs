// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::BTreeMap;
use std::rc::Rc;

#[derive(Clone)]
pub(super) enum BindingOrigin {
    RootPath { path: String, required: bool },
    LocalOnly,
}

pub(super) enum Scope {
    Root,
    Component {
        attrs: BTreeMap<String, BindingOrigin>,
        require_global_fallback: bool,
    },
    Loop {
        item: String,
        origin: BindingOrigin,
        parent: Rc<Scope>,
    },
}

pub(super) fn resolve_path(scope: &Rc<Scope>, path: &str) -> BindingOrigin {
    let (first, rest) = path
        .split_once('.')
        .map_or((path, None), |(first, rest)| (first, Some(rest)));
    let mut current = scope.as_ref();
    loop {
        match current {
            Scope::Root => {
                return BindingOrigin::RootPath {
                    path: path.to_string(),
                    required: true,
                };
            }
            Scope::Component {
                attrs,
                require_global_fallback,
            } => {
                let Some(origin) = attrs.get(first) else {
                    return BindingOrigin::RootPath {
                        path: path.to_string(),
                        required: *require_global_fallback,
                    };
                };
                return append_origin(origin, rest);
            }
            Scope::Loop {
                item,
                origin,
                parent,
            } => {
                if item == first {
                    return append_origin(origin, rest);
                }
                current = parent.as_ref();
            }
        }
    }
}

pub(super) fn has_component_scope(scope: &Rc<Scope>) -> bool {
    let mut current = scope.as_ref();
    loop {
        match current {
            Scope::Root => return false,
            Scope::Component { .. } => return true,
            Scope::Loop { parent, .. } => current = parent.as_ref(),
        }
    }
}

pub(super) fn array_item_path(mut collection: String) -> String {
    collection.reserve(2);
    collection.push_str("[]");
    collection
}

fn append_origin(origin: &BindingOrigin, rest: Option<&str>) -> BindingOrigin {
    match origin {
        BindingOrigin::RootPath { path, required } => {
            let mut resolved =
                String::with_capacity(path.len() + rest.map_or(0, |value| value.len() + 1));
            resolved.push_str(path);
            if let Some(rest) = rest {
                resolved.push('.');
                resolved.push_str(rest);
            }
            BindingOrigin::RootPath {
                path: resolved,
                required: *required,
            }
        }
        BindingOrigin::LocalOnly => BindingOrigin::LocalOnly,
    }
}
