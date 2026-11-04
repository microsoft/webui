// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Components the entry page loads at startup.

use std::collections::HashSet;
use webui_protocol::{web_ui_fragment::Fragment, WebUIProtocol};

/// Components the entry page loads at startup: every component that `entry`
/// reaches without crossing a lazy region or a `<route>`.
///
/// Lazy bodies load with their packs and route components with route
/// partials, so neither counts. Region loaders skip startup components because
/// the page has already loaded their code. The walk is iterative and visits
/// each fragment record once. The set is empty when `entry` is not a fragment
/// of `protocol`.
#[must_use]
pub fn startup_components<'a>(protocol: &'a WebUIProtocol, entry: &str) -> HashSet<&'a str> {
    let mut components = HashSet::new();
    let Some((entry, _)) = protocol.fragments.get_key_value(entry) else {
        return components;
    };
    let mut visited = HashSet::new();
    let mut pending = vec![entry.as_str()];
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        let Some(list) = protocol.fragments.get(id) else {
            continue;
        };
        for fragment in &list.fragments {
            match fragment.fragment.as_ref() {
                Some(Fragment::Component(component)) => {
                    components.insert(component.fragment_id.as_str());
                    pending.push(&component.fragment_id);
                }
                Some(Fragment::ForLoop(repeat)) => pending.push(&repeat.fragment_id),
                Some(Fragment::IfCond(condition)) if !condition.lazy => {
                    pending.push(&condition.fragment_id);
                }
                Some(Fragment::Attribute(attribute)) if !attribute.template.is_empty() => {
                    pending.push(&attribute.template);
                }
                _ => {}
            }
        }
    }
    components
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use webui_protocol::{
        lazy_fragment_id, ConditionExpr, FragmentList, WebUIFragment, WebUiFragmentRoute,
    };

    fn list(fragments: Vec<WebUIFragment>) -> FragmentList {
        FragmentList {
            fragments,
            ..Default::default()
        }
    }

    #[test]
    fn startup_stops_at_lazy_regions_and_routes() {
        let region = lazy_fragment_id("app-shell", 0);
        let route = WebUiFragmentRoute {
            path: "/reports".to_owned(),
            fragment_id: "reports-page".to_owned(),
            exact: true,
            ..Default::default()
        };
        let protocol = WebUIProtocol::new(HashMap::from([
            (
                "index.html".to_owned(),
                list(vec![
                    WebUIFragment::component("site-header"),
                    WebUIFragment::component("app-shell"),
                    WebUIFragment::route_from(route),
                ]),
            ),
            (
                "site-header".to_owned(),
                list(vec![WebUIFragment::if_cond(
                    ConditionExpr::identifier("signedIn"),
                    "site-header::if",
                )]),
            ),
            (
                "site-header::if".to_owned(),
                list(vec![WebUIFragment::component("user-badge")]),
            ),
            ("user-badge".to_owned(), FragmentList::default()),
            (
                "app-shell".to_owned(),
                list(vec![WebUIFragment::lazy_if(
                    ConditionExpr::identifier("open"),
                    &region,
                )]),
            ),
            (
                region,
                list(vec![
                    WebUIFragment::component("user-badge"),
                    WebUIFragment::component("settings-panel"),
                ]),
            ),
            ("settings-panel".to_owned(), FragmentList::default()),
            (
                "reports-page".to_owned(),
                list(vec![WebUIFragment::component("report-chart")]),
            ),
            ("report-chart".to_owned(), FragmentList::default()),
        ]));

        let mut startup: Vec<&str> = startup_components(&protocol, "index.html")
            .into_iter()
            .collect();
        startup.sort_unstable();
        assert_eq!(startup, ["app-shell", "site-header", "user-badge"]);
        assert!(startup_components(&protocol, "missing.html").is_empty());
    }
}
