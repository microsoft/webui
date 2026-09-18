// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Route and outlet rendering helpers.
//!
//! Free functions for escaping HTML attribute values and selecting the best
//! matching route among sibling route fragments.

use crate::route_matcher;
use crate::route_matcher::CompiledRouteIndex;
use crate::{ResponseWriter, Result};
use webui_protocol::{web_ui_fragment::Fragment, WebUIFragment, WebUiFragmentRoute};

/// Write attributes needed by the client before a route partial resolves.
///
/// Cache-related attributes (cache-tags, invalidates) and query are omitted
/// from the DOM because they're delivered via the inline SSR chain JSON.
/// Pending/error/keep-alive remain available on unmatched placeholders so the
/// client can select destination boundary UI before the partial arrives.
pub(crate) fn write_route_navigation_attrs(
    writer: &mut dyn ResponseWriter,
    route: &WebUiFragmentRoute,
) -> Result<()> {
    if !route.pending_component.is_empty() {
        writer.write(" pending=\"")?;
        writer.write(&route.pending_component)?;
        writer.write("\"")?;
    }
    if !route.error_component.is_empty() {
        writer.write(" error=\"")?;
        writer.write(&route.error_component)?;
        writer.write("\"")?;
    }
    if route.keep_alive {
        writer.write(" keep-alive")?;
    }
    Ok(())
}

/// Escape HTML special characters in an attribute value and write directly to the writer.
///
/// Escapes `&`, `"`, `<`, and `>` using HTML entities. Writes unescaped
/// segments directly to avoid intermediate string allocation.
pub(crate) fn write_escaped_state_attr(writer: &mut dyn ResponseWriter, value: &str) -> Result<()> {
    let mut last = 0;

    for (index, ch) in value.char_indices() {
        let escaped = match ch {
            '&' => Some("&amp;"),
            '"' => Some("&quot;"),
            '<' => Some("&lt;"),
            '>' => Some("&gt;"),
            _ => None,
        };

        if let Some(entity) = escaped {
            if last < index {
                writer.write(&value[last..index])?;
            }
            writer.write(entity)?;
            last = index + ch.len_utf8();
        }
    }

    if last < value.len() {
        writer.write(&value[last..])?;
    }

    Ok(())
}

/// Pre-scan sibling route fragments and return the best match info.
///
/// Picks the route with the highest specificity (most literal segments).
/// This ensures `/contacts/add` (2 literals) beats `/contacts/:id` (1 literal + 1 param).
///
/// `route_base` is used to resolve relative paths (starting with `./`).
///
/// Request segmentation is deferred until a route fragment is actually seen:
/// every record entry calls this, and the overwhelming majority of records —
/// component bodies, conditions, loop bodies — carry no routes at all, so a
/// route-free record must not pay for a segment vector.
pub(crate) fn find_best_route_match(
    fragments: &[WebUIFragment],
    request_path: &str,
    route_base: &str,
    route_index: &CompiledRouteIndex,
) -> Option<(String, route_matcher::RouteMatch)> {
    let (_, route, matched) =
        find_best_route_match_ref(fragments, request_path, route_base, route_index)?;
    Some((route.fragment_id.clone(), matched))
}

/// Select a sibling route without allocating an owned copy of its component ID.
pub(crate) fn find_best_route_match_ref<'a>(
    fragments: &'a [WebUIFragment],
    request_path: &str,
    route_base: &str,
    route_index: &CompiledRouteIndex,
) -> Option<(usize, &'a WebUiFragmentRoute, route_matcher::RouteMatch)> {
    let mut best: Option<(usize, &WebUiFragmentRoute, route_matcher::RouteMatch)> = None;
    let mut request_segments: Option<Vec<&str>> = None;

    for (index, item) in fragments.iter().enumerate() {
        if let Some(Fragment::Route(route_frag)) = item.fragment.as_ref() {
            let segments = request_segments
                .get_or_insert_with(|| route_matcher::split_request_path(request_path));
            if let Some(m) = route_matcher::match_route_indexed_with_segments(
                route_index,
                &route_frag.path,
                route_base,
                segments,
                route_frag.exact,
            ) {
                let is_better = best
                    .as_ref()
                    .is_none_or(|(_, _, prev)| m.specificity > prev.specificity);

                if is_better {
                    best = Some((index, route_frag, m));
                }
            }
        }
    }

    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use webui_protocol::{FragmentList, WebUIProtocol};
    use webui_test_utils::test_json;

    crate::define_string_response_writer!(RouteWriter, output);

    fn route(path: &str) -> WebUIFragment {
        WebUIFragment {
            fragment: Some(Fragment::Route(WebUiFragmentRoute {
                path: path.to_owned(),
                fragment_id: "detail".to_owned(),
                exact: true,
                ..Default::default()
            })),
        }
    }

    fn protocol() -> WebUIProtocol {
        WebUIProtocol::new(HashMap::from([
            (
                "entry".to_owned(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("START"),
                        route("/contacts/:id"),
                        route("/contacts/add"),
                        route("./notes/:note"),
                    ],
                    contains_boundary: false,
                },
            ),
            (
                "detail".to_owned(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("CARD")],
                    contains_boundary: false,
                },
            ),
        ]))
    }

    #[test]
    fn borrowed_route_selection_keeps_specificity_parameters_and_source_addresses() {
        let protocol = protocol();
        let index = CompiledRouteIndex::new(&protocol);
        let fragments = &protocol.fragments["entry"].fragments;
        for (path, base, expected_index, parameter) in [
            ("/contacts/add", "", 2, None),
            ("/contacts/42", "", 1, Some(("id", "42"))),
            (
                "/contacts/42/notes/7",
                "/contacts/42",
                3,
                Some(("note", "7")),
            ),
        ] {
            let (selected, route, matched) =
                find_best_route_match_ref(fragments, path, base, &index)
                    .unwrap_or_else(|| panic!("route should match {path}"));
            assert_eq!(selected, expected_index);
            let Some(Fragment::Route(source)) = fragments[selected].fragment.as_ref() else {
                panic!("selected fragment should be a route");
            };
            assert!(std::ptr::eq(source, route));
            let (key, owned) = find_best_route_match(fragments, path, base, &index)
                .unwrap_or_else(|| panic!("owned route selection should match"));
            assert_eq!(key, route.fragment_id);
            assert_eq!(owned.consumed_segments, matched.consumed_segments);
            assert_eq!(owned.params, matched.params);
            if let Some((name, value)) = parameter {
                assert_eq!(matched.params.get(name).map(String::as_str), Some(value));
            }
        }
    }

    #[test]
    fn compact_cursor_keeps_component_key_selection_for_shared_route_targets() -> Result<()> {
        let protocol = crate::Protocol::new(protocol());
        let mut writer = RouteWriter::with_capacity(1024);
        crate::WebUIHandler::new().render(
            &protocol,
            &test_json!({}),
            &crate::RenderOptions::new("entry", "/contacts/add"),
            &mut writer,
        )?;
        assert_eq!(writer.output.matches("CARD").count(), 3);
        assert_eq!(writer.output.matches(" active>").count(), 3);
        Ok(())
    }
}
