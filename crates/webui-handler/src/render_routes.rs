// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Immutable route addresses retained by the shared continuation cursor.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use webui_protocol::{web_ui_fragment::Fragment, WebUIProtocol, WebUiFragmentRoute};
use yoke::Yoke;

use crate::state_view::Provenance;

pub(crate) struct RoutePreparation {
    pub(crate) roots: HashMap<usize, u32>,
    pub(crate) provenance: Provenance,
}

pub(crate) struct RenderRoutes {
    routes: Yoke<RouteRecords<'static>, Arc<WebUIProtocol>>,
    children: Vec<Range<u32>>,
}

#[derive(yoke::Yokeable)]
struct RouteRecords<'a> {
    routes: Vec<&'a WebUiFragmentRoute>,
}

impl RenderRoutes {
    pub(crate) fn new(protocol: &Arc<WebUIProtocol>) -> (Self, RoutePreparation) {
        let mut preparation = RoutePreparation {
            roots: HashMap::new(),
            provenance: Provenance::Omit,
        };
        let mut children = Vec::new();
        let routes = Yoke::attach_to_cart(Arc::clone(protocol), |protocol| {
            let mut routes = Vec::new();
            for list in protocol.fragments.values() {
                for fragment in &list.fragments {
                    match fragment.fragment.as_ref() {
                        Some(Fragment::Route(route)) => {
                            // Only preparation uses this identity; the immutable
                            // protocol keeps every referenced route at its address.
                            #[allow(clippy::cast_possible_truncation)]
                            preparation
                                .roots
                                .insert(std::ptr::from_ref(route).addr(), routes.len() as u32);
                            routes.push(route);
                        }
                        Some(Fragment::Render(_)) => preparation.provenance = Provenance::Track,
                        _ => {}
                    }
                }
            }
            let mut index = 0;
            while let Some(route) = routes.get(index).copied() {
                #[allow(clippy::cast_possible_truncation)]
                let start = routes.len() as u32;
                routes.extend(route.children.iter());
                #[allow(clippy::cast_possible_truncation)]
                children.push(start..routes.len() as u32);
                index += 1;
            }
            RouteRecords { routes }
        });
        (Self { routes, children }, preparation)
    }

    pub(crate) fn get(&self, index: u32) -> Option<&WebUiFragmentRoute> {
        self.routes.get().routes.get(index as usize).copied()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &WebUiFragmentRoute> {
        self.routes.get().routes.iter().copied()
    }

    pub(crate) fn children(&self, index: u32) -> Option<Range<u32>> {
        self.children.get(index as usize).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use webui_protocol::{FragmentList, WebUIFragment};

    #[test]
    fn route_addresses_borrow_original_nested_protocol_storage() {
        let protocol = Arc::new(WebUIProtocol::new(HashMap::from([(
            "entry".into(),
            FragmentList {
                fragments: vec![WebUIFragment {
                    fragment: Some(Fragment::Route(WebUiFragmentRoute {
                        path: "/".into(),
                        children: vec![WebUiFragmentRoute {
                            path: "./child".into(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    })),
                }],
                contains_boundary: false,
            },
        )])));
        let (routes, preparation) = RenderRoutes::new(&protocol);
        let Some(Fragment::Route(original)) =
            protocol.fragments["entry"].fragments[0].fragment.as_ref()
        else {
            panic!("expected route");
        };
        let root = preparation.roots[&std::ptr::from_ref(original).addr()];
        assert!(routes
            .get(root)
            .is_some_and(|route| std::ptr::eq(route, original)));
        let Some(children) = routes.children(root) else {
            panic!("expected child range");
        };
        assert_eq!(children.len(), 1);
        assert!(routes
            .get(children.start)
            .is_some_and(|route| std::ptr::eq(route, &original.children[0])));
        assert_eq!(routes.iter().count(), 2);
        drop(protocol);
        assert_eq!(
            routes.get(children.start).map(|route| route.path.as_str()),
            Some("./child")
        );
    }

    #[test]
    fn record_order_does_not_change_route_identity_or_compiled_matching() {
        use crate::route_matcher::{
            match_route_indexed_with_segments, split_request_path, CompiledRouteIndex,
        };

        let protocol = Arc::new(WebUIProtocol::new(
            [("z-owner", "/z/:id"), ("a-owner", "/a")]
                .into_iter()
                .map(|(id, path)| {
                    (
                        id.to_owned(),
                        FragmentList {
                            fragments: vec![
                                WebUIFragment::raw("prefix"),
                                WebUIFragment {
                                    fragment: Some(Fragment::Route(WebUiFragmentRoute {
                                        path: path.to_owned(),
                                        children: vec![WebUiFragmentRoute {
                                            path: "./child/:child?".to_owned(),
                                            ..Default::default()
                                        }],
                                        ..Default::default()
                                    })),
                                },
                            ],
                            contains_boundary: false,
                        },
                    )
                })
                .collect(),
        ));
        let (routes, preparation) = RenderRoutes::new(&protocol);
        assert_eq!(preparation.roots.len(), 2);
        assert_eq!(routes.iter().count(), 4);
        for record in protocol.fragments.values() {
            let Some(Fragment::Route(original)) = record.fragments[1].fragment.as_ref() else {
                panic!("expected an original route");
            };
            let slot = preparation.roots[&std::ptr::from_ref(original).addr()];
            assert!(routes
                .get(slot)
                .is_some_and(|route| std::ptr::eq(route, original)));
            let children = routes
                .children(slot)
                .unwrap_or_else(|| panic!("prepared route must have a child range"));
            assert_eq!(children.len(), 1);
            assert!(routes
                .get(children.start)
                .is_some_and(|route| std::ptr::eq(route, &original.children[0])));
        }
        let original = CompiledRouteIndex::new(&protocol);
        let prepared = CompiledRouteIndex::from_routes(routes.iter());
        for (template, base, path, exact) in [
            ("/a", "/", "/a", true),
            ("/z/:id", "/", "/z/42", true),
            ("./child/:child?", "/a", "/a/child", true),
            ("./child/:child?", "/z/42", "/z/42/child/7", true),
            ("/a", "/", "/z/42", true),
        ] {
            let segments = split_request_path(path);
            let matched = |index| {
                match_route_indexed_with_segments(index, template, base, &segments, exact).map(
                    |matched| {
                        (
                            matched.params,
                            matched.specificity,
                            matched.consumed_segments,
                        )
                    },
                )
            };
            assert_eq!(matched(&prepared), matched(&original));
        }
    }
}
