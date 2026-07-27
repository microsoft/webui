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

/// Write pending/error attributes for a route fragment.
///
/// Cache-related attributes (cache-tags, invalidates) and query/keep-alive
/// are omitted from the DOM — they're delivered via the inline SSR chain JSON.
/// Only pending/error are kept as DOM attributes because the client needs them
/// for descendant fallback scanning on first navigation into unvisited subtrees.
pub(crate) fn write_route_pending_attrs(
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
pub(crate) fn find_best_route_match(
    fragments: &[WebUIFragment],
    request_path: &str,
    route_base: &str,
    route_index: &CompiledRouteIndex,
) -> Option<(String, route_matcher::RouteMatch)> {
    let mut best: Option<(String, route_matcher::RouteMatch)> = None;

    let request_segments = route_matcher::split_request_path(request_path);

    for item in fragments {
        if let Some(Fragment::Route(route_frag)) = item.fragment.as_ref() {
            if let Some(m) = route_matcher::match_route_indexed_with_segments(
                route_index,
                &route_frag.path,
                route_base,
                &request_segments,
                route_frag.exact,
            ) {
                let is_better = best
                    .as_ref()
                    .is_none_or(|(_, prev)| m.specificity > prev.specificity);

                if is_better {
                    let key = route_frag.fragment_id.clone();
                    best = Some((key, m));
                }
            }
        }
    }

    best
}
