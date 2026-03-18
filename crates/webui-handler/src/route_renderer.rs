// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Route and outlet rendering helpers.
//!
//! Free functions for emitting state attributes on route component elements,
//! escaping HTML attribute values, and selecting the best matching route
//! among sibling route fragments.

use crate::route_matcher;
use crate::{ResponseWriter, Result};
use serde_json::Value;
use webui_protocol::{web_ui_fragment::Fragment, WebUIFragment};

/// Emit top-level state values as HTML attributes on a route component element.
///
/// This ensures FAST hydration reads the correct values from DOM attributes
/// instead of using the component's default `@attr` values.
///
/// Scalar values (string, number, bool) are emitted as individual kebab-case
/// attributes. The full state (including arrays/objects) is also emitted as a
/// `data-state` JSON attribute so components can read complex state during hydration.
pub(crate) fn emit_state_attributes(state: &Value, writer: &mut dyn ResponseWriter) -> Result<()> {
    let map = match state.as_object() {
        Some(m) => m,
        None => return Ok(()),
    };

    // Emit scalar values as individual attributes
    for (key, value) in map {
        let val_str = match value {
            Value::String(s) => std::borrow::Cow::Borrowed(s.as_str()),
            Value::Number(n) => std::borrow::Cow::Owned(n.to_string()),
            Value::Bool(true) => std::borrow::Cow::Borrowed("true"),
            Value::Bool(false) => std::borrow::Cow::Borrowed("false"),
            _ => continue,
        };
        let attr_name = super::camel_to_kebab(key);
        writer.write(" ")?;
        writer.write(&attr_name)?;
        writer.write("=\"")?;
        write_escaped_state_attr(writer, val_str.as_ref())?;
        writer.write("\"")?;
    }

    // Emit full state as data-state for complex values (arrays, objects)
    let has_complex = map.values().any(|v| v.is_array() || v.is_object());
    if has_complex {
        let json_str = state.to_string();
        writer.write(" data-state=\"")?;
        write_escaped_state_attr(writer, &json_str)?;
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
