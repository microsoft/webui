// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Route component inventory management for incremental route rendering.
//!
//! These helpers walk the normal render fragment graph. The request-aware path is
//! route-aware but state-agnostic: it follows the active route chain for the
//! current request path, while conservatively traversing `if`, `for`, and
//! attribute-template edges without evaluating runtime state.

use crate::route_matcher;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use webui_protocol::{web_ui_fragment::Fragment, WebUIFragment, WebUIFragmentRoute, WebUIProtocol};

// ── Component Inventory ─────────────────────────────────────────────────

/// FNV-1a hash mod 256 — deterministic bit position for a component name.
/// Must match the client-side implementation in `@microsoft/webui-router`.
fn component_bit_position(name: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in name.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash % 256
}

/// Check if a component is present in an inventory bitmask.
fn has_component(inventory: &[u8], name: &str) -> bool {
    let bit = component_bit_position(name);
    let byte_idx = (bit / 8) as usize;
    let bit_idx = bit % 8;
    byte_idx < inventory.len() && inventory[byte_idx] & (1 << bit_idx) != 0
}

/// Parse a hex string into bytes for the inventory bitmask.
pub fn parse_inventory(hex: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    let mut chars = hex.bytes();
    while let (Some(hi), Some(lo)) = (chars.next(), chars.next()) {
        if let (Some(hi_nibble), Some(lo_nibble)) = (hex_nibble(hi), hex_nibble(lo)) {
            bytes.push((hi_nibble << 4) | lo_nibble);
        }
    }
    bytes
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Encode an inventory bitmask as a hex string.
pub fn encode_inventory(inv: &[u8]) -> String {
    inv.iter()
        .fold(String::with_capacity(inv.len() * 2), |mut acc, b| {
            use std::fmt::Write;
            let _ = write!(acc, "{b:02x}");
            acc
        })
}

/// Walk the protocol fragment graph from `entry_id` and return the names of
/// all components the route needs that are NOT in the client's inventory.
///
/// `entry_id` is the route's component name (e.g., `"cb-page-group"`).
/// `inventory` is the client's bitmask of already-loaded components.
///
/// Returns `(needed_names, updated_inventory_hex)`.
pub fn get_needed_components(
    protocol: &WebUIProtocol,
    entry_id: &str,
    inventory_hex: &str,
) -> (Vec<String>, String) {
    let component_names = collect_inventoryable_components(protocol, entry_id, None, true);
    filter_needed_components(&component_names, inventory_hex)
}

/// Walk the protocol from the persistent `entry_id` and return the component
/// templates needed for the current `request_path`.
///
/// The traversal is route-aware but state-agnostic:
///
/// - sibling `<route>` fragments are pruned to the single best match for the
///   request path
/// - nested route groups are traversed recursively via that same rule
/// - `if`, `for`, and attribute-template edges are still followed
///   conservatively, without evaluating runtime state
///
/// Returns `(needed_names, updated_inventory_hex)`.
pub fn get_needed_components_for_request(
    protocol: &WebUIProtocol,
    entry_id: &str,
    request_path: &str,
    inventory_hex: &str,
) -> (Vec<String>, String) {
    let component_names =
        collect_inventoryable_components(protocol, entry_id, Some(request_path), false);
    filter_needed_components(&component_names, inventory_hex)
}

/// Filter an inventoryable component set against the client's inventory bitmask.
///
/// Returns the missing component names and the updated inventory hex string.
#[must_use]
pub fn filter_needed_components(
    component_names: &HashSet<String>,
    inventory_hex: &str,
) -> (Vec<String>, String) {
    let mut updated_inv = parse_inventory(inventory_hex);
    updated_inv.resize(32, 0);

    let mut ordered_names: Vec<&String> = component_names.iter().collect();
    ordered_names.sort_unstable();

    let mut needed = Vec::with_capacity(ordered_names.len());
    for name in ordered_names {
        if has_component(&updated_inv, name) {
            continue;
        }

        let bit = component_bit_position(name);
        let byte_idx = (bit / 8) as usize;
        let bit_idx = bit % 8;
        if byte_idx < updated_inv.len() {
            updated_inv[byte_idx] |= 1 << bit_idx;
        }
        needed.push(name.clone());
    }

    (needed, encode_inventory(&updated_inv))
}

/// Get the f-template HTML strings needed for a route that the client doesn't
/// already have.
///
/// Walks the fragment graph from `entry_id`, identifies needed components (not
/// in the client's inventory), and returns their f-template HTML from the
/// protocol's `component_templates` map.
///
/// Returns `(templates: Vec<(name, html)>, updated_inventory_hex)`.
pub fn get_route_templates(
    protocol: &WebUIProtocol,
    entry_id: &str,
    inventory_hex: &str,
) -> (Vec<(String, String)>, String) {
    let (needed_names, updated_inv) = get_needed_components(protocol, entry_id, inventory_hex);

    let templates: Vec<(String, String)> = needed_names
        .iter()
        .filter_map(|name| {
            protocol
                .component_templates
                .get(name)
                .map(|tmpl| (name.clone(), tmpl.clone()))
        })
        .collect();

    (templates, updated_inv)
}

/// Get the f-template HTML strings needed for the active route chain rooted at
/// `entry_id` for the current `request_path`.
///
/// Returns `(templates: Vec<(name, html)>, updated_inventory_hex)`.
pub fn get_route_templates_for_request(
    protocol: &WebUIProtocol,
    entry_id: &str,
    request_path: &str,
    inventory_hex: &str,
) -> (Vec<(String, String)>, String) {
    let (needed_names, updated_inv) =
        get_needed_components_for_request(protocol, entry_id, request_path, inventory_hex);

    let templates: Vec<(String, String)> = needed_names
        .iter()
        .filter_map(|name| {
            protocol
                .component_templates
                .get(name)
                .map(|tmpl| (name.clone(), tmpl.clone()))
        })
        .collect();

    (templates, updated_inv)
}

#[derive(Debug)]
struct QueuedFragment {
    id: String,
    inventoryable: bool,
    /// Base path for resolving relative route paths at this level.
    route_base: String,
}

/// Walk the fragment graph from `entry_id` and collect all inventoryable component names.
///
/// Uses an iterative stack-based traversal. When `request_path` is provided,
/// the walk follows only the best-matching route at each nesting level (route-aware).
/// Without a request path, all route branches are followed conservatively.
///
/// Components are marked `inventoryable` when they have a corresponding entry
/// in `protocol.component_templates` — these are the components whose f-template
/// HTML the client may need during navigation.
fn collect_inventoryable_components(
    protocol: &WebUIProtocol,
    entry_id: &str,
    request_path: Option<&str>,
    root_inventoryable: bool,
) -> HashSet<String> {
    let mut visited_fragments = HashSet::new();
    let mut component_ids = HashSet::new();
    let mut stack = vec![QueuedFragment {
        id: entry_id.to_string(),
        inventoryable: root_inventoryable,
        route_base: "/".to_string(),
    }];

    while let Some(queued) = stack.pop() {
        if queued.id.is_empty() {
            continue;
        }

        if queued.inventoryable {
            component_ids.insert(queued.id.clone());
        }

        if !visited_fragments.insert(queued.id.clone()) {
            continue;
        }

        let Some(frag_list) = protocol.fragments.get(&queued.id) else {
            continue;
        };

        let matched_route = request_path
            .and_then(|path| find_best_route_match(&frag_list.fragments, path, &queued.route_base));

        for frag in &frag_list.fragments {
            match frag.fragment.as_ref() {
                Some(Fragment::Component(component)) => {
                    stack.push(QueuedFragment {
                        id: component.fragment_id.clone(),
                        inventoryable: true,
                        route_base: queued.route_base.clone(),
                    });
                }
                Some(Fragment::ForLoop(for_loop)) => {
                    stack.push(QueuedFragment {
                        id: for_loop.fragment_id.clone(),
                        inventoryable: false,
                        route_base: queued.route_base.clone(),
                    });
                }
                Some(Fragment::IfCond(if_cond)) => {
                    stack.push(QueuedFragment {
                        id: if_cond.fragment_id.clone(),
                        inventoryable: false,
                        route_base: queued.route_base.clone(),
                    });
                }
                Some(Fragment::Attribute(attr)) if !attr.template.is_empty() => {
                    stack.push(QueuedFragment {
                        id: attr.template.clone(),
                        inventoryable: false,
                        route_base: queued.route_base.clone(),
                    });
                }
                Some(Fragment::Route(route_frag)) => {
                    let is_selected = matched_route
                        .as_ref()
                        .is_some_and(|(best_key, _)| best_key == route_fragment_key(route_frag));
                    if is_selected && !route_frag.fragment_id.is_empty() {
                        // Compute new route base from consumed segments
                        let child_route_base = if let Some((_, ref rm)) = matched_route {
                            if let Some(path) = request_path {
                                route_matcher::compute_route_base(path, rm.consumed_segments)
                            } else {
                                queued.route_base.clone()
                            }
                        } else {
                            queued.route_base.clone()
                        };

                        stack.push(QueuedFragment {
                            id: route_frag.fragment_id.clone(),
                            inventoryable: protocol
                                .component_templates
                                .contains_key(&route_frag.fragment_id),
                            route_base: child_route_base.clone(),
                        });

                        // Walk nested child routes to find the next matched level.
                        // This mirrors the handler's outlet rendering: match children
                        // against the request path and follow the matched chain.
                        if !route_frag.children.is_empty() {
                            if let Some(path) = request_path {
                                walk_route_children(
                                    &route_frag.children,
                                    path,
                                    &child_route_base,
                                    protocol,
                                    &mut stack,
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    component_ids
}

/// Walk nested route children to find matched routes and add their
/// components to the inventory stack. Mirrors the handler's outlet rendering.
fn walk_route_children(
    children: &[WebUIFragmentRoute],
    request_path: &str,
    route_base: &str,
    protocol: &WebUIProtocol,
    stack: &mut Vec<QueuedFragment>,
) {
    let mut current = children;
    let mut base = route_base.to_string();

    loop {
        let mut best: Option<(usize, route_matcher::RouteMatch)> = None;
        for (idx, child) in current.iter().enumerate() {
            let resolved = route_matcher::resolve_route_path(&child.path, &base);
            if let Some(m) = route_matcher::match_single_route(&resolved, request_path, child.exact)
            {
                let is_better = best
                    .as_ref()
                    .is_none_or(|(_, prev)| m.specificity > prev.specificity);
                if is_better {
                    best = Some((idx, m));
                }
            }
        }

        let Some((idx, ref rm)) = best else { break };
        let matched = &current[idx];
        if matched.fragment_id.is_empty() {
            break;
        }

        let child_base = route_matcher::compute_route_base(request_path, rm.consumed_segments);

        stack.push(QueuedFragment {
            id: matched.fragment_id.clone(),
            inventoryable: protocol
                .component_templates
                .contains_key(&matched.fragment_id),
            route_base: child_base.clone(),
        });

        if matched.children.is_empty() {
            break;
        }
        current = &matched.children;
        base = child_base;
    }
}

/// Pre-scan sibling route fragments and return the best match info.
///
/// `route_base` is used to resolve relative paths (starting with `./`).
fn find_best_route_match(
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
                    best = Some((route_fragment_key(route_frag).to_string(), m));
                }
            }
        }
    }

    best
}

fn route_fragment_key(route_frag: &WebUIFragmentRoute) -> &str {
    route_frag.fragment_id.as_str()
}

/// Walk the fragment graph following matched routes and collect all route
/// parameters from every level of the active route chain.
///
/// This is used by the dev server to inject nested route params into state.
pub fn collect_nested_route_params(
    protocol: &WebUIProtocol,
    entry_id: &str,
    request_path: &str,
) -> HashMap<String, String> {
    let mut all_params = HashMap::new();
    let mut visited_fragments = HashSet::new();
    let mut stack = vec![QueuedFragment {
        id: entry_id.to_string(),
        inventoryable: false,
        route_base: "/".to_string(),
    }];

    while let Some(queued) = stack.pop() {
        if queued.id.is_empty() || !visited_fragments.insert(queued.id.clone()) {
            continue;
        }

        let Some(frag_list) = protocol.fragments.get(&queued.id) else {
            continue;
        };

        let matched_route =
            find_best_route_match(&frag_list.fragments, request_path, &queued.route_base);

        for frag in &frag_list.fragments {
            match frag.fragment.as_ref() {
                Some(Fragment::Component(component)) => {
                    stack.push(QueuedFragment {
                        id: component.fragment_id.clone(),
                        inventoryable: false,
                        route_base: queued.route_base.clone(),
                    });
                }
                Some(Fragment::Route(route_frag)) => {
                    let is_selected = matched_route
                        .as_ref()
                        .is_some_and(|(best_key, _)| best_key == route_fragment_key(route_frag));
                    if is_selected && !route_frag.fragment_id.is_empty() {
                        if let Some((_, ref rm)) = matched_route {
                            // Collect params from this route level
                            all_params.extend(rm.params.clone());

                            let child_route_base = route_matcher::compute_route_base(
                                request_path,
                                rm.consumed_segments,
                            );

                            stack.push(QueuedFragment {
                                id: route_frag.fragment_id.clone(),
                                inventoryable: false,
                                route_base: child_route_base.clone(),
                            });

                            // Walk nested children to collect params from deeper levels
                            collect_params_from_children(
                                &route_frag.children,
                                request_path,
                                &child_route_base,
                                &mut all_params,
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }

    all_params
}

/// Recursively collect route params from nested route children.
fn collect_params_from_children(
    children: &[WebUIFragmentRoute],
    request_path: &str,
    route_base: &str,
    all_params: &mut HashMap<String, String>,
) {
    let mut current = children;
    let mut base = route_base.to_string();

    loop {
        let mut best: Option<(usize, route_matcher::RouteMatch)> = None;
        for (idx, child) in current.iter().enumerate() {
            let resolved = route_matcher::resolve_route_path(&child.path, &base);
            if let Some(m) = route_matcher::match_single_route(&resolved, request_path, child.exact)
            {
                let is_better = best
                    .as_ref()
                    .is_none_or(|(_, prev)| m.specificity > prev.specificity);
                if is_better {
                    best = Some((idx, m));
                }
            }
        }

        let Some((idx, rm)) = best else { break };
        all_params.extend(rm.params);
        let matched = &current[idx];
        if matched.children.is_empty() {
            break;
        }
        let child_base = route_matcher::compute_route_base(request_path, rm.consumed_segments);
        current = &matched.children;
        base = child_base;
    }
}

// ── Partial Response ────────────────────────────────────────────────

/// Produce a complete JSON partial response for client-side navigation.
///
/// Combines route templates, inventory, and matched route chain into a single
/// JSON value. Host servers include this directly in their response alongside
/// the application `state`.
///
/// Returns a `serde_json::Value` object with fields:
/// - `state`: the application state passed through
/// - `templates`: array of f-template HTML strings the client needs
/// - `inventory`: updated hex bitmask
/// - `path`: the request path
/// - `chain`: matched route chain array
#[must_use]
pub fn render_partial(
    protocol: &WebUIProtocol,
    state: Value,
    entry_id: &str,
    request_path: &str,
    inventory_hex: &str,
) -> Value {
    // Get needed templates (filtered by client inventory)
    let (templates, updated_inv) =
        get_route_templates_for_request(protocol, entry_id, request_path, inventory_hex);

    // Build the matched route chain
    let chain = collect_route_chain(protocol, entry_id, request_path);

    // Assemble the response
    let tmpl_array: Vec<Value> = templates
        .into_iter()
        .map(|(_, html)| Value::String(html))
        .collect();

    let chain_array = Value::Array(chain.iter().map(RouteChainEntry::to_json).collect());

    let mut result = serde_json::Map::with_capacity(5);
    result.insert("state".into(), state);
    result.insert("templates".into(), Value::Array(tmpl_array));
    result.insert("inventory".into(), Value::String(updated_inv));
    result.insert("path".into(), Value::String(request_path.to_string()));
    result.insert("chain".into(), chain_array);
    Value::Object(result)
}

// ── Route Chain ─────────────────────────────────────────────────────

/// A single entry in the matched route chain, one per nesting level.
#[derive(Debug, Clone)]
pub struct RouteChainEntry {
    /// Component tag name for this route level.
    pub component: String,
    /// The route path pattern (as declared in the template).
    pub path: String,
    /// Bound route parameters at this level.
    pub params: HashMap<String, String>,
    /// Whether this route requires an exact match.
    pub exact: bool,
}

impl RouteChainEntry {
    /// Serialize this entry to a JSON value ready for inclusion in a partial response.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut obj = serde_json::Map::with_capacity(4);
        obj.insert("component".into(), Value::String(self.component.clone()));
        obj.insert("path".into(), Value::String(self.path.clone()));
        if !self.params.is_empty() {
            let params_obj: serde_json::Map<String, Value> = self
                .params
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect();
            obj.insert("params".into(), Value::Object(params_obj));
        }
        if self.exact {
            obj.insert("exact".into(), Value::Bool(true));
        }
        Value::Object(obj)
    }
}

/// Collect the matched route chain as a JSON array value.
///
/// Convenience wrapper around [`collect_route_chain`] that serializes the
/// result into a `serde_json::Value::Array` ready for inclusion in a JSON
/// partial response. Host servers in any language can include this directly
/// without reimplementing the serialization.
#[must_use]
pub fn collect_route_chain_json(
    protocol: &WebUIProtocol,
    entry_id: &str,
    request_path: &str,
) -> Value {
    let chain = collect_route_chain(protocol, entry_id, request_path);
    Value::Array(chain.iter().map(RouteChainEntry::to_json).collect())
}

/// Collect the matched route chain for a request path.
///
/// Walks the fragment graph from `entry_id`, follows the matched route at
/// each nesting level, and returns a chain entry per matched level.
pub fn collect_route_chain(
    protocol: &WebUIProtocol,
    entry_id: &str,
    request_path: &str,
) -> Vec<RouteChainEntry> {
    let mut chain = Vec::new();
    let mut visited_fragments = HashSet::new();
    let mut stack = vec![QueuedFragment {
        id: entry_id.to_string(),
        inventoryable: false,
        route_base: "/".to_string(),
    }];

    while let Some(queued) = stack.pop() {
        if queued.id.is_empty() || !visited_fragments.insert(queued.id.clone()) {
            continue;
        }

        let Some(frag_list) = protocol.fragments.get(&queued.id) else {
            continue;
        };

        let matched_route =
            find_best_route_match(&frag_list.fragments, request_path, &queued.route_base);

        for frag in &frag_list.fragments {
            match frag.fragment.as_ref() {
                Some(Fragment::Component(component)) => {
                    stack.push(QueuedFragment {
                        id: component.fragment_id.clone(),
                        inventoryable: false,
                        route_base: queued.route_base.clone(),
                    });
                }
                Some(Fragment::Route(route_frag)) => {
                    let is_selected = matched_route
                        .as_ref()
                        .is_some_and(|(best_key, _)| best_key == route_fragment_key(route_frag));
                    if is_selected && !route_frag.fragment_id.is_empty() {
                        if let Some((_, ref rm)) = matched_route {
                            chain.push(RouteChainEntry {
                                component: route_frag.fragment_id.clone(),
                                path: route_frag.path.clone(),
                                params: rm.params.clone(),
                                exact: route_frag.exact,
                            });

                            let child_route_base = route_matcher::compute_route_base(
                                request_path,
                                rm.consumed_segments,
                            );

                            stack.push(QueuedFragment {
                                id: route_frag.fragment_id.clone(),
                                inventoryable: false,
                                route_base: child_route_base.clone(),
                            });

                            // Walk nested children iteratively
                            collect_chain_from_children(
                                &route_frag.children,
                                request_path,
                                &child_route_base,
                                &mut chain,
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }

    chain
}

/// Iteratively collect chain entries from nested route children.
fn collect_chain_from_children(
    children: &[WebUIFragmentRoute],
    request_path: &str,
    route_base: &str,
    chain: &mut Vec<RouteChainEntry>,
) {
    let mut pending: Vec<(&[WebUIFragmentRoute], String)> =
        vec![(children, route_base.to_string())];

    while let Some((current, base)) = pending.pop() {
        let mut best: Option<(usize, route_matcher::RouteMatch)> = None;
        for (idx, child) in current.iter().enumerate() {
            let resolved = route_matcher::resolve_route_path(&child.path, &base);
            if let Some(m) = route_matcher::match_single_route(&resolved, request_path, child.exact)
            {
                let is_better = best
                    .as_ref()
                    .is_none_or(|(_, prev)| m.specificity > prev.specificity);
                if is_better {
                    best = Some((idx, m));
                }
            }
        }

        if let Some((idx, rm)) = best {
            let matched = &current[idx];
            chain.push(RouteChainEntry {
                component: matched.fragment_id.clone(),
                path: matched.path.clone(),
                params: rm.params,
                exact: matched.exact,
            });
            if !matched.children.is_empty() {
                let child_base =
                    route_matcher::compute_route_base(request_path, rm.consumed_segments);
                pending.push((&matched.children, child_base));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use webui_protocol::{FragmentList, WebUIFragment, WebUiFragmentRoute};

    #[test]
    fn test_component_bit_position_deterministic() {
        let pos1 = component_bit_position("my-component");
        let pos2 = component_bit_position("my-component");
        assert_eq!(pos1, pos2);
        assert!(pos1 < 256);
    }

    #[test]
    fn test_has_component_present() {
        let mut inv = vec![0u8; 32];
        let bit = component_bit_position("test-comp");
        let byte_idx = (bit / 8) as usize;
        let bit_idx = bit % 8;
        inv[byte_idx] |= 1 << bit_idx;
        assert!(has_component(&inv, "test-comp"));
    }

    #[test]
    fn test_has_component_absent() {
        let inv = vec![0u8; 32];
        assert!(!has_component(&inv, "test-comp"));
    }

    #[test]
    fn test_parse_encode_inventory_roundtrip() {
        let original = vec![0xABu8, 0xCD, 0xEF, 0x01];
        let hex = encode_inventory(&original);
        let decoded = parse_inventory(&hex);
        assert_eq!(original, decoded);
    }

    #[test]
    fn test_get_needed_components_empty_inventory() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-card")],
            },
        );
        fragments.insert(
            "my-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("card")],
            },
        );

        let protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        let (needed, _inv) = get_needed_components(&protocol, "app-shell", "");
        assert!(needed.contains(&"app-shell".to_string()));
        assert!(needed.contains(&"my-card".to_string()));
    }

    #[test]
    fn test_get_route_templates() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "my-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>page</p>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        protocol
            .component_templates
            .insert("my-page".to_string(), "<p>page</p>".to_string());

        let (templates, _inv) = get_route_templates(&protocol, "my-page", "");
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].0, "my-page");
    }

    #[test]
    fn test_get_needed_components_skips_inventory() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-card")],
            },
        );
        fragments.insert(
            "my-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("card")],
            },
        );

        let protocol = WebUIProtocol::with_tokens(fragments, Vec::new());

        let (_needed, inv_hex) = get_needed_components(&protocol, "app-shell", "");
        let (needed2, _) = get_needed_components(&protocol, "app-shell", &inv_hex);
        assert!(needed2.is_empty());
    }

    #[test]
    fn test_get_needed_components_skips_control_fragments() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::if_cond(
                        webui_protocol::ConditionExpr::identifier("showNav"),
                        "if-shell",
                    ),
                    WebUIFragment::for_loop("item", "items", "for-items"),
                    WebUIFragment::attribute_template("title", "attr-title"),
                ],
            },
        );
        fragments.insert(
            "if-shell".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-category-nav")],
            },
        );
        fragments.insert(
            "for-items".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-product-card")],
            },
        );
        fragments.insert(
            "attr-title".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("Products")],
            },
        );
        fragments.insert(
            "mp-category-nav".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<nav></nav>")],
            },
        );
        fragments.insert(
            "mp-product-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<article></article>")],
            },
        );

        let protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        let needed: HashSet<String> = get_needed_components(&protocol, "app-shell", "")
            .0
            .into_iter()
            .collect();

        assert!(needed.contains("app-shell"));
        assert!(needed.contains("mp-category-nav"));
        assert!(needed.contains("mp-product-card"));
        assert!(!needed.contains("if-shell"));
        assert!(!needed.contains("for-items"));
        assert!(!needed.contains("attr-title"));
    }

    #[test]
    fn test_get_needed_components_for_request_includes_shell_and_matched_route_chain() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-app")],
            },
        );
        fragments.insert(
            "mp-app".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::component("mp-category-nav"),
                    WebUIFragment::route_from(WebUiFragmentRoute {
                        path: "/search/:category".into(),
                        fragment_id: "mp-search-page".into(),
                        exact: true,
                        ..Default::default()
                    }),
                    WebUIFragment::route_from(WebUiFragmentRoute {
                        path: "/product/:handle".into(),
                        fragment_id: "mp-product-page".into(),
                        exact: true,
                        ..Default::default()
                    }),
                ],
            },
        );
        fragments.insert(
            "mp-category-nav".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<nav></nav>")],
            },
        );
        fragments.insert(
            "mp-search-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-product-grid")],
            },
        );
        fragments.insert(
            "mp-product-grid".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>grid</div>")],
            },
        );
        fragments.insert(
            "mp-product-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-product-detail")],
            },
        );
        fragments.insert(
            "mp-product-detail".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>detail</div>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        protocol.component_templates.insert(
            "mp-search-page".to_string(),
            "<mp-search-page></mp-search-page>".to_string(),
        );
        protocol.component_templates.insert(
            "mp-product-page".to_string(),
            "<mp-product-page></mp-product-page>".to_string(),
        );

        let (needed, _inv) =
            get_needed_components_for_request(&protocol, "index.html", "/search/shirts", "");
        let needed: HashSet<String> = needed.into_iter().collect();

        assert!(needed.contains("mp-app"));
        assert!(needed.contains("mp-category-nav"));
        assert!(needed.contains("mp-search-page"));
        assert!(needed.contains("mp-product-grid"));
        assert!(!needed.contains("mp-product-page"));
        assert!(!needed.contains("mp-product-detail"));
    }

    #[test]
    fn test_get_needed_components_for_request_follows_nested_route_chain() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-app")],
            },
        );
        fragments.insert(
            "mp-app".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/account/*rest".into(),
                    fragment_id: "mp-account-shell".into(),
                    exact: false,
                    ..Default::default()
                })],
            },
        );
        fragments.insert(
            "mp-account-shell".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::component("mp-account-nav"),
                    WebUIFragment::route_from(WebUiFragmentRoute {
                        path: "/account/profile".into(),
                        fragment_id: "mp-profile-page".into(),
                        exact: true,
                        ..Default::default()
                    }),
                    WebUIFragment::route_from(WebUiFragmentRoute {
                        path: "/account/orders/:id".into(),
                        fragment_id: "mp-order-page".into(),
                        exact: true,
                        ..Default::default()
                    }),
                ],
            },
        );
        fragments.insert(
            "mp-account-nav".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<nav></nav>")],
            },
        );
        fragments.insert(
            "mp-profile-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<profile></profile>")],
            },
        );
        fragments.insert(
            "mp-order-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-order-detail")],
            },
        );
        fragments.insert(
            "mp-order-detail".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<detail></detail>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        protocol.component_templates.insert(
            "mp-account-shell".to_string(),
            "<mp-account-shell></mp-account-shell>".to_string(),
        );
        protocol.component_templates.insert(
            "mp-profile-page".to_string(),
            "<mp-profile-page></mp-profile-page>".to_string(),
        );
        protocol.component_templates.insert(
            "mp-order-page".to_string(),
            "<mp-order-page></mp-order-page>".to_string(),
        );

        let (needed, _inv) =
            get_needed_components_for_request(&protocol, "index.html", "/account/orders/42", "");
        let needed: HashSet<String> = needed.into_iter().collect();

        assert!(needed.contains("mp-app"));
        assert!(needed.contains("mp-account-shell"));
        assert!(needed.contains("mp-account-nav"));
        assert!(needed.contains("mp-order-page"));
        assert!(needed.contains("mp-order-detail"));
        assert!(!needed.contains("mp-profile-page"));
    }

    #[test]
    fn test_get_needed_components_for_request_over_ships_if_and_loop_within_matched_route() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-app")],
            },
        );
        fragments.insert(
            "mp-app".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/search".into(),
                    fragment_id: "mp-search-page".into(),
                    exact: true,
                    ..Default::default()
                })],
            },
        );
        fragments.insert(
            "mp-search-page".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::if_cond(
                        webui_protocol::ConditionExpr::identifier("showFilters"),
                        "if-filters",
                    ),
                    WebUIFragment::for_loop("item", "items", "item-loop"),
                ],
            },
        );
        fragments.insert(
            "if-filters".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-filter-panel")],
            },
        );
        fragments.insert(
            "item-loop".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-item-card")],
            },
        );
        fragments.insert(
            "mp-filter-panel".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<filters></filters>")],
            },
        );
        fragments.insert(
            "mp-item-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<item></item>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        protocol.component_templates.insert(
            "mp-search-page".to_string(),
            "<mp-search-page></mp-search-page>".to_string(),
        );

        let (needed, _inv) =
            get_needed_components_for_request(&protocol, "index.html", "/search", "");
        let needed: HashSet<String> = needed.into_iter().collect();

        assert!(needed.contains("mp-app"));
        assert!(needed.contains("mp-search-page"));
        assert!(needed.contains("mp-filter-panel"));
        assert!(needed.contains("mp-item-card"));
    }

    #[test]
    fn test_get_needed_components_for_request_respects_inventory() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-app")],
            },
        );
        fragments.insert(
            "mp-app".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/search".into(),
                    fragment_id: "mp-search-page".into(),
                    exact: true,
                    ..Default::default()
                })],
            },
        );
        fragments.insert(
            "mp-search-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-product-grid")],
            },
        );
        fragments.insert(
            "mp-product-grid".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<grid></grid>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        protocol.component_templates.insert(
            "mp-search-page".to_string(),
            "<mp-search-page></mp-search-page>".to_string(),
        );

        let (_needed, inventory) =
            get_needed_components_for_request(&protocol, "index.html", "/search", "");
        let (needed_again, _) =
            get_needed_components_for_request(&protocol, "index.html", "/search", &inventory);
        assert!(needed_again.is_empty());
    }

    #[test]
    fn test_get_route_templates_for_request_returns_only_active_route_templates() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-app")],
            },
        );
        fragments.insert(
            "mp-app".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/search".into(),
                    fragment_id: "mp-search-page".into(),
                    exact: true,
                    ..Default::default()
                })],
            },
        );
        fragments.insert(
            "mp-search-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-product-grid")],
            },
        );
        fragments.insert(
            "mp-product-grid".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<grid></grid>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        protocol.component_templates.insert(
            "mp-app".to_string(),
            "<f-template id=app></f-template>".to_string(),
        );
        protocol.component_templates.insert(
            "mp-search-page".to_string(),
            "<f-template id=search></f-template>".to_string(),
        );
        protocol.component_templates.insert(
            "mp-product-grid".to_string(),
            "<f-template id=grid></f-template>".to_string(),
        );

        let (templates, _inv) =
            get_route_templates_for_request(&protocol, "index.html", "/search", "");

        assert_eq!(templates.len(), 3);
        assert_eq!(templates[0].0, "mp-app");
        assert_eq!(templates[1].0, "mp-product-grid");
        assert_eq!(templates[2].0, "mp-search-page");
    }
}
