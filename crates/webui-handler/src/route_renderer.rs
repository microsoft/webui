// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Route and outlet rendering helpers.
//!
//! Free functions for escaping HTML attribute values and selecting the best
//! matching route among sibling route fragments.

use crate::route_matcher;
use crate::{ResponseWriter, Result};
use webui_protocol::{web_ui_fragment::Fragment, WebUIFragment, WebUiFragmentRoute};

/// Write comma-separated items directly to the writer without allocating a joined string.
fn write_comma_separated(writer: &mut dyn ResponseWriter, items: &[String]) -> Result<()> {
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            writer.write(",")?;
        }
        writer.write(item)?;
    }
    Ok(())
}

/// Write cache/invalidation/pending/error attributes for a route fragment.
///
/// Shared by `process_route`, `process_outlet` (matched child), and
/// `process_outlet` (hidden siblings) to avoid divergence.
pub(crate) fn write_route_cache_attrs(
    writer: &mut dyn ResponseWriter,
    route: &WebUiFragmentRoute,
) -> Result<()> {
    if !route.cache_tags.is_empty() {
        writer.write(" cache-tags=\"")?;
        write_comma_separated(writer, &route.cache_tags)?;
        writer.write("\"")?;
    }
    if !route.invalidates.is_empty() {
        writer.write(" invalidates=\"")?;
        write_comma_separated(writer, &route.invalidates)?;
        writer.write("\"")?;
    }
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
) -> Option<(String, route_matcher::RouteMatch)> {
    let mut best: Option<(String, route_matcher::RouteMatch)> = None;

    for item in fragments {
        if let Some(Fragment::Route(route_frag)) = item.fragment.as_ref() {
            let resolved_path = route_matcher::resolve_route_path(&route_frag.path, route_base);
            if let Some(m) =
                route_matcher::match_single_route(&resolved_path, request_path, route_frag.exact)
            {
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
