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

/// Build a deterministic component-name → bit-index map from the protocol.
///
/// Derives names from fragment keys (hyphenated = custom element) since that
/// is the source of truth regardless of whether a plugin populated
/// `protocol.components`. Components are sorted alphabetically; index =
/// position in that order.
pub fn build_component_index(protocol: &WebUIProtocol) -> HashMap<String, u32> {
    let mut names: HashSet<&String> = HashSet::new();
    for key in protocol.fragments.keys() {
        if key.contains('-') {
            names.insert(key);
        }
    }
    let mut sorted: Vec<&String> = names.into_iter().collect();
    sorted.sort_unstable();
    sorted
        .iter()
        .enumerate()
        .map(|(i, n)| {
            // Index count bounded by component registry size, well within u32 range
            #[allow(clippy::cast_possible_truncation)]
            let idx = i as u32;
            ((*n).clone(), idx)
        })
        .collect()
}

/// Check if a component's bit is set in the inventory bitfield.
fn has_component(inventory: &[u8], index: u32) -> bool {
    let byte_idx = (index / 8) as usize;
    let bit_idx = index % 8;
    byte_idx < inventory.len() && inventory[byte_idx] & (1 << bit_idx) != 0
}

/// Set a component's bit in the inventory bitfield.
fn set_component(inventory: &mut Vec<u8>, index: u32) {
    let byte_idx = (index / 8) as usize;
    let bit_idx = index % 8;
    if byte_idx >= inventory.len() {
        inventory.resize(byte_idx + 1, 0);
    }
    inventory[byte_idx] |= 1 << bit_idx;
}

/// Parse a hex-encoded inventory string into bytes.
pub fn parse_inventory(hex: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    let mut chars = hex.bytes();
    while let (Some(hi), Some(lo)) = (chars.next(), chars.next()) {
        if let (Some(h), Some(l)) = (hex_nibble(hi), hex_nibble(lo)) {
            bytes.push((h << 4) | l);
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

/// Encode an inventory bitfield as a hex string.
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
/// Returns `(needed_names, updated_inventory_hex)`.
pub fn get_needed_components(
    protocol: &WebUIProtocol,
    entry_id: &str,
    inventory_hex: &str,
) -> (Vec<String>, String) {
    let component_names = collect_inventoryable_components(protocol, entry_id, None, true);
    let index = build_component_index(protocol);
    filter_needed_components(&component_names, inventory_hex, &index)
}

/// Walk the protocol from the persistent `entry_id` and return the component
/// templates needed for the current `request_path`.
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
    let index = build_component_index(protocol);
    filter_needed_components(&component_names, inventory_hex, &index)
}

/// Filter components against the client's inventory bitfield using sequential indices.
/// Zero collisions — each component has a unique bit.
///
/// Returns the missing component names and the updated inventory hex string.
#[must_use]
pub fn filter_needed_components(
    component_names: &HashSet<String>,
    inventory_hex: &str,
    index: &HashMap<String, u32>,
) -> (Vec<String>, String) {
    let client_inv = parse_inventory(inventory_hex);
    let mut updated_inv = client_inv.clone();

    let mut ordered_names: Vec<&String> = component_names.iter().collect();
    ordered_names.sort_unstable();

    let mut needed = Vec::with_capacity(ordered_names.len());
    for name in ordered_names {
        if let Some(&idx) = index.get(name.as_str()) {
            if !has_component(&client_inv, idx) {
                needed.push(name.clone());
            }
            set_component(&mut updated_inv, idx);
        } else {
            // Component exists in the fragment graph but has no index entry
            // (no protocol.components record). It can't be tracked in the
            // bitfield, so we must always send it.
            needed.push(name.clone());
        }
    }

    (needed, encode_inventory(&updated_inv))
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
/// in `protocol.components` with a non-empty template — these are the components
/// whose f-template HTML the client may need during navigation.
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
                                .components
                                .get(&route_frag.fragment_id)
                                .is_some_and(|c| !c.template.is_empty()),
                            route_base: child_route_base.clone(),
                        });

                        // Include pending/error components in the inventory
                        // so their templates are available on the client.
                        if !route_frag.pending_component.is_empty() {
                            stack.push(QueuedFragment {
                                id: route_frag.pending_component.clone(),
                                inventoryable: protocol
                                    .components
                                    .get(&route_frag.pending_component)
                                    .is_some_and(|c| !c.template.is_empty()),
                                route_base: child_route_base.clone(),
                            });
                        }
                        if !route_frag.error_component.is_empty() {
                            stack.push(QueuedFragment {
                                id: route_frag.error_component.clone(),
                                inventoryable: protocol
                                    .components
                                    .get(&route_frag.error_component)
                                    .is_some_and(|c| !c.template.is_empty()),
                                route_base: child_route_base.clone(),
                            });
                        }

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
                .components
                .get(&matched.fragment_id)
                .is_some_and(|c| !c.template.is_empty()),
            route_base: child_base.clone(),
        });

        // Include pending/error components in the inventory
        if !matched.pending_component.is_empty() {
            stack.push(QueuedFragment {
                id: matched.pending_component.clone(),
                inventoryable: protocol
                    .components
                    .get(&matched.pending_component)
                    .is_some_and(|c| !c.template.is_empty()),
                route_base: child_base.clone(),
            });
        }
        if !matched.error_component.is_empty() {
            stack.push(QueuedFragment {
                id: matched.error_component.clone(),
                inventoryable: protocol
                    .components
                    .get(&matched.error_component)
                    .is_some_and(|c| !c.template.is_empty()),
                route_base: child_base.clone(),
            });
        }

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

/// Resolve `{param}` placeholders in a list of tag templates.
///
/// Replaces `{paramName}` with the actual value from `params`.
/// Tags without placeholders are returned as-is. Missing params
/// result in the placeholder being left unresolved (defensive).
fn resolve_tag_templates(templates: &[String], params: &HashMap<String, String>) -> Vec<String> {
    let mut resolved = Vec::with_capacity(templates.len());
    for template in templates {
        if !template.contains('{') {
            resolved.push(template.clone());
            continue;
        }
        let mut result = String::with_capacity(template.len());
        let mut rest = template.as_str();
        while let Some(open) = rest.find('{') {
            result.push_str(&rest[..open]);
            rest = &rest[open + 1..];
            if let Some(close) = rest.find('}') {
                let name = &rest[..close];
                if let Some(value) = params.get(name) {
                    result.push_str(value);
                } else {
                    result.push('{');
                    result.push_str(name);
                    result.push('}');
                }
                rest = &rest[close + 1..];
            } else {
                result.push('{');
            }
        }
        result.push_str(rest);
        resolved.push(result);
    }
    resolved
}

/// Produce a JSON partial response for client-side navigation.
///
/// Returns the matched route chain, templates, inventory, and cache tags.
/// **State is not included** — the caller is responsible for adding
/// application state to the response (e.g. as a top-level `"state"` field
/// for non-streaming, or as NDJSON Chunk 2 for streaming).
///
/// Returns a `serde_json::Value` object with fields:
/// - `templateStyles`: module CSS definition tags for inventory-new components (empty for Link/Style)
/// - `templates`: client template payloads the client doesn't already have (inventory-filtered)
/// - `inventory`: updated hex bitmask
/// - `path`: the request path
/// - `chain`: matched route chain array
/// - `cacheTags`: resolved cache tags from the full route chain (union of all levels)
#[must_use]
pub fn render_partial(
    protocol: &WebUIProtocol,
    entry_id: &str,
    request_path: &str,
    inventory_hex: &str,
) -> Value {
    let (needed_names, updated_inv) =
        get_needed_components_for_request(protocol, entry_id, request_path, inventory_hex);

    let mut chain = collect_route_chain(protocol, entry_id, request_path);

    // Resolve cache tags and invalidation templates with accumulated params.
    let mut accumulated_params: HashMap<String, String> = HashMap::new();
    let mut all_resolved_tags: Vec<String> = Vec::new();

    for entry in &mut chain {
        for (k, v) in &entry.params {
            accumulated_params.insert(k.clone(), v.clone());
        }
        entry.cache_tags = resolve_tag_templates(&entry.cache_tags, &accumulated_params);
        entry.invalidates = resolve_tag_templates(&entry.invalidates, &accumulated_params);
        all_resolved_tags.extend(entry.cache_tags.iter().cloned());
    }

    let tag_refs: Vec<&str> = needed_names.iter().map(|s| s.as_str()).collect();
    let (style_array, tmpl_array) = collect_component_assets(protocol, &tag_refs);

    let chain_array = Value::Array(chain.iter().map(RouteChainEntry::to_json).collect());

    let mut result = serde_json::Map::with_capacity(6);
    result.insert("templateStyles".into(), Value::Array(style_array));
    result.insert("templates".into(), Value::Array(tmpl_array));
    result.insert("inventory".into(), Value::String(updated_inv));
    result.insert("path".into(), Value::String(request_path.to_string()));
    result.insert("chain".into(), chain_array);
    if !all_resolved_tags.is_empty() {
        let mut seen = HashSet::new();
        let deduped: Vec<Value> = all_resolved_tags
            .into_iter()
            .filter(|t| seen.insert(t.clone()))
            .map(Value::String)
            .collect();
        result.insert("cacheTags".into(), Value::Array(deduped));
    }
    Value::Object(result)
}

/// Produce a JSON response for a POST mutation action.
///
/// Walks the matched route chain, resolves `invalidates` tag templates
/// with actual param values, and returns them alongside the application
/// state. Host servers call this for `POST` requests to route paths.
///
/// Returns a `serde_json::Value` object with fields:
/// - `state`: the application state passed through
/// - `invalidateTags`: resolved invalidation tags from the matched leaf route
/// - `path`: the request path
#[must_use]
pub fn render_action_response(
    protocol: &WebUIProtocol,
    state: Value,
    entry_id: &str,
    request_path: &str,
) -> Value {
    let chain = collect_route_chain(protocol, entry_id, request_path);

    // Accumulate params across the chain for tag resolution
    let mut accumulated_params: HashMap<String, String> = HashMap::new();
    let mut all_invalidates: Vec<String> = Vec::new();

    for entry in &chain {
        for (k, v) in &entry.params {
            accumulated_params.insert(k.clone(), v.clone());
        }
        let resolved = resolve_tag_templates(&entry.invalidates, &accumulated_params);
        all_invalidates.extend(resolved);
    }

    // Deduplicate while preserving order
    let mut seen = HashSet::new();
    let deduped: Vec<Value> = all_invalidates
        .into_iter()
        .filter(|t| seen.insert(t.clone()))
        .map(Value::String)
        .collect();

    let mut result = serde_json::Map::with_capacity(3);
    result.insert("state".into(), state);
    result.insert("invalidateTags".into(), Value::Array(deduped));
    result.insert("path".into(), Value::String(request_path.to_string()));
    Value::Object(result)
}

/// Return compiled templates and CSS for specific components by tag name.
///
/// This supports on-demand loading of components that aren't part of the
/// route tree (e.g., dialogs, popovers) — the client calls this before
/// creating the element so templates + styles are registered without FOUC.
///
/// Uses the same inventory bitfield as partial navigation to avoid sending
/// templates the client already has.
#[must_use]
pub fn render_component_templates(
    protocol: &WebUIProtocol,
    component_tags: &[&str],
    inventory_hex: &str,
) -> Value {
    let requested: HashSet<String> = component_tags.iter().map(|s| s.to_string()).collect();
    let index = build_component_index(protocol);
    let (needed, updated_inv) = filter_needed_components(&requested, inventory_hex, &index);

    let tag_refs: Vec<&str> = needed.iter().map(|s| s.as_str()).collect();
    let (style_array, tmpl_array) = collect_component_assets(protocol, &tag_refs);

    let mut result = serde_json::Map::with_capacity(3);
    result.insert("templateStyles".into(), Value::Array(style_array));
    result.insert("templates".into(), Value::Array(tmpl_array));
    result.insert("inventory".into(), Value::String(updated_inv));
    Value::Object(result)
}

/// Shared helper: collect templates and module CSS styles for a set of component tags.
fn collect_component_assets(protocol: &WebUIProtocol, tags: &[&str]) -> (Vec<Value>, Vec<Value>) {
    let mut style_array = Vec::new();
    let mut tmpl_array = Vec::new();

    // Sort for deterministic output (reproducible responses, cache keys)
    let mut sorted_tags: Vec<&str> = tags.to_vec();
    sorted_tags.sort_unstable();

    for tag in sorted_tags {
        let Some(component) = protocol.components.get(tag) else {
            continue;
        };
        if component.template.is_empty() {
            continue;
        }
        if !component.css.is_empty() {
            let mut s = String::with_capacity(40 + tag.len() + component.css.len());
            s.push_str("<style type=\"module\" specifier=\"");
            s.push_str(tag);
            s.push_str("\">");
            s.push_str(&component.css);
            s.push_str("</style>");
            style_array.push(Value::String(s));
        }
        tmpl_array.push(Value::String(component.template.clone()));
    }

    (style_array, tmpl_array)
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
    /// Comma-separated allowlist of query parameters forwarded as attributes.
    pub allowed_query: String,
    /// When true, the router keeps the component alive across navigations.
    pub keep_alive: bool,
    /// Cache tag templates from the proto (e.g. `["thread:{threadId}", "inbox"]`).
    pub cache_tags: Vec<String>,
    /// Invalidation tag templates from the proto.
    pub invalidates: Vec<String>,
    /// Component tag name for pending/loading UI.
    pub pending_component: String,
    /// Component tag name for error boundary UI.
    pub error_component: String,
}

impl RouteChainEntry {
    /// Serialize this entry to a JSON value ready for inclusion in a partial response.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut obj = serde_json::Map::with_capacity(9);
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
        if !self.allowed_query.is_empty() {
            obj.insert(
                "allowedQuery".into(),
                Value::String(self.allowed_query.clone()),
            );
        }
        if self.keep_alive {
            obj.insert("keepAlive".into(), Value::Bool(true));
        }
        if !self.pending_component.is_empty() {
            obj.insert(
                "pendingComponent".into(),
                Value::String(self.pending_component.clone()),
            );
        }
        if !self.error_component.is_empty() {
            obj.insert(
                "errorComponent".into(),
                Value::String(self.error_component.clone()),
            );
        }
        if !self.invalidates.is_empty() {
            obj.insert(
                "invalidates".into(),
                Value::Array(
                    self.invalidates
                        .iter()
                        .map(|s| Value::String(s.clone()))
                        .collect(),
                ),
            );
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
                                allowed_query: route_frag.allowed_query.clone(),
                                keep_alive: route_frag.keep_alive,
                                cache_tags: route_frag.cache_tags.clone(),
                                invalidates: route_frag.invalidates.clone(),
                                pending_component: route_frag.pending_component.clone(),
                                error_component: route_frag.error_component.clone(),
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
                allowed_query: matched.allowed_query.clone(),
                keep_alive: matched.keep_alive,
                cache_tags: matched.cache_tags.clone(),
                invalidates: matched.invalidates.clone(),
                pending_component: matched.pending_component.clone(),
                error_component: matched.error_component.clone(),
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
    fn test_parse_encode_inventory_roundtrip() {
        let original = vec![0xABu8, 0xCD, 0xEF, 0x01];
        let hex = encode_inventory(&original);
        let decoded = parse_inventory(&hex);
        assert_eq!(original, decoded);
    }

    #[test]
    fn test_bitfield_set_and_check() {
        let mut inv = vec![0u8; 4];
        set_component(&mut inv, 0);
        set_component(&mut inv, 7);
        set_component(&mut inv, 8);
        set_component(&mut inv, 15);
        assert!(has_component(&inv, 0));
        assert!(has_component(&inv, 7));
        assert!(has_component(&inv, 8));
        assert!(has_component(&inv, 15));
        assert!(!has_component(&inv, 1));
        assert!(!has_component(&inv, 9));
    }

    #[test]
    fn test_filter_needed_components_no_false_positives() {
        // With sequential indices, no two components share a bit
        let mut index = HashMap::new();
        index.insert("email-message".to_string(), 0);
        index.insert("o-button".to_string(), 1);
        index.insert("o-avatar".to_string(), 2);

        let mut names = HashSet::new();
        names.insert("email-message".to_string());
        names.insert("o-button".to_string());

        // Only o-button (index 1) is loaded — bit 1 set
        let mut inv = vec![0u8; 1];
        set_component(&mut inv, 1); // o-button
        let inv_hex = encode_inventory(&inv);

        let (needed, _) = filter_needed_components(&names, &inv_hex, &index);
        assert_eq!(needed.len(), 1);
        assert!(
            needed.contains(&"email-message".to_string()),
            "email-message must be needed: {needed:?}"
        );
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

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        protocol
            .components
            .entry("app-shell".to_string())
            .or_default()
            .template = "<t></t>".to_string();
        protocol
            .components
            .entry("my-card".to_string())
            .or_default()
            .template = "<t></t>".to_string();

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
                        keep_alive: false,
                        ..Default::default()
                    }),
                    WebUIFragment::route_from(WebUiFragmentRoute {
                        path: "/product/:handle".into(),
                        fragment_id: "mp-product-page".into(),
                        exact: true,
                        keep_alive: false,
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
        protocol
            .components
            .entry("mp-search-page".to_string())
            .or_default()
            .template = "<mp-search-page></mp-search-page>".to_string();
        protocol
            .components
            .entry("mp-product-page".to_string())
            .or_default()
            .template = "<mp-product-page></mp-product-page>".to_string();

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
                    keep_alive: false,
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
                        keep_alive: false,
                        ..Default::default()
                    }),
                    WebUIFragment::route_from(WebUiFragmentRoute {
                        path: "/account/orders/:id".into(),
                        fragment_id: "mp-order-page".into(),
                        exact: true,
                        keep_alive: false,
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
        protocol
            .components
            .entry("mp-account-shell".to_string())
            .or_default()
            .template = "<mp-account-shell></mp-account-shell>".to_string();
        protocol
            .components
            .entry("mp-profile-page".to_string())
            .or_default()
            .template = "<mp-profile-page></mp-profile-page>".to_string();
        protocol
            .components
            .entry("mp-order-page".to_string())
            .or_default()
            .template = "<mp-order-page></mp-order-page>".to_string();

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
                    keep_alive: false,
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
        protocol
            .components
            .entry("mp-search-page".to_string())
            .or_default()
            .template = "<mp-search-page></mp-search-page>".to_string();

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
                    keep_alive: false,
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
        for name in ["mp-app", "mp-search-page", "mp-product-grid"] {
            protocol
                .components
                .entry(name.to_string())
                .or_default()
                .template = format!("<f-template id=\"{name}\"></f-template>");
        }

        let (_needed, inventory) =
            get_needed_components_for_request(&protocol, "index.html", "/search", "");
        let (needed_again, _) =
            get_needed_components_for_request(&protocol, "index.html", "/search", &inventory);
        assert!(needed_again.is_empty());
    }

    #[test]
    fn test_render_partial_separates_module_styles_from_templates() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-page")],
            },
        );
        fragments.insert(
            "my-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>page</p>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        let component = protocol
            .components
            .entry("my-page".to_string())
            .or_default();
        component.template =
            "(function(){window.__webui_templates['my-page']={h:'<p>page</p>'};})();".to_string();
        component.css = ".page{color:red}".to_string();

        let partial = render_partial(&protocol, "index.html", "/", "");
        let styles = partial["templateStyles"]
            .as_array()
            .expect("templateStyles should be an array");
        assert_eq!(styles.len(), 1);
        assert!(
            styles[0]
                .as_str()
                .unwrap_or_default()
                .contains("specifier=\"my-page\""),
            "module style should carry the component specifier"
        );
        assert!(
            styles[0]
                .as_str()
                .unwrap_or_default()
                .contains(".page{color:red}"),
            "module style should contain the CSS content"
        );

        // templates should contain only the clean JS IIFE
        let templates = partial["templates"]
            .as_array()
            .expect("templates should be an array");
        assert_eq!(templates.len(), 1);
        assert!(
            !templates[0].as_str().unwrap_or_default().contains("<style"),
            "template should not contain any style tags"
        );
        assert!(
            templates[0]
                .as_str()
                .unwrap_or_default()
                .starts_with("(function()"),
            "template should be a raw JS IIFE"
        );
    }

    #[test]
    fn test_render_partial_link_strategy_has_empty_template_styles() {
        // Link-strategy components have css_href but no css content.
        // templateStyles should be empty; templates should contain the IIFE.
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-page")],
            },
        );
        fragments.insert(
            "my-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>page</p>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        let component = protocol
            .components
            .entry("my-page".to_string())
            .or_default();
        component.template = "(function(){})();".to_string();
        component.css_href = "/my-page.css".to_string();
        // css is empty — Link strategy stores href, not content

        let partial = render_partial(&protocol, "index.html", "/", "");
        let styles = partial["templateStyles"]
            .as_array()
            .expect("templateStyles should be an array");
        let templates = partial["templates"]
            .as_array()
            .expect("templates should be an array");

        assert!(
            styles.is_empty(),
            "Link strategy should produce empty templateStyles: {styles:?}"
        );
        assert_eq!(templates.len(), 1, "should include the template IIFE");
    }

    #[test]
    fn test_render_partial_style_strategy_has_empty_template_styles() {
        // Style-strategy components have CSS embedded in the template HTML
        // (as <style>...</style>), not in component.css.
        // templateStyles should be empty; templates should contain the IIFE.
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-page")],
            },
        );
        fragments.insert(
            "my-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>page</p>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        let component = protocol
            .components
            .entry("my-page".to_string())
            .or_default();
        // Style strategy: CSS is inside the template HTML, not in component.css
        component.template =
            "(function(){var w=window.__webui_templates;w['my-page']={h:'<style>.p{color:red}</style><p>page</p>'};})();"
                .to_string();
        // css is empty for Style strategy

        let partial = render_partial(&protocol, "index.html", "/", "");
        let styles = partial["templateStyles"]
            .as_array()
            .expect("templateStyles should be an array");
        let templates = partial["templates"]
            .as_array()
            .expect("templates should be an array");

        assert!(
            styles.is_empty(),
            "Style strategy should produce empty templateStyles: {styles:?}"
        );
        assert_eq!(templates.len(), 1, "should include the template IIFE");
        assert!(
            templates[0]
                .as_str()
                .unwrap_or_default()
                .contains("<style>"),
            "Style strategy template should contain inline <style> tag"
        );
    }

    #[test]
    fn test_render_partial_empty_styles_for_no_css_components() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-page")],
            },
        );
        fragments.insert(
            "my-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>page</p>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        let component = protocol
            .components
            .entry("my-page".to_string())
            .or_default();
        component.template = "(function(){})();".to_string();
        // No CSS — simulates Link or Style mode

        let partial = render_partial(&protocol, "index.html", "/", "");
        let styles = partial["templateStyles"]
            .as_array()
            .expect("templateStyles should be an array");
        assert!(
            styles.is_empty(),
            "templateStyles should be empty when components have no CSS"
        );
    }

    #[test]
    fn test_render_partial_sends_styles_even_when_templates_filtered_by_inventory() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-page")],
            },
        );
        fragments.insert(
            "my-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>page</p>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        let component = protocol
            .components
            .entry("my-page".to_string())
            .or_default();
        component.template = "(function(){})();".to_string();
        component.css = ".page{color:red}".to_string();

        // First call to establish inventory
        let partial1 = render_partial(&protocol, "index.html", "/", "");
        let inv = partial1["inventory"].as_str().unwrap_or_default();
        assert!(!inv.is_empty());

        // Second call with the inventory — both templates and styles should be empty
        // because the inventory covers this component.  The SSR handler emits all
        // module style definitions in <head> for inventoried components, so the
        // client already has the CSS definition.
        let partial2 = render_partial(&protocol, "index.html", "/", inv);
        let styles = partial2["templateStyles"]
            .as_array()
            .expect("templateStyles should be an array");
        let templates = partial2["templates"]
            .as_array()
            .expect("templates should be an array");

        assert!(
            templates.is_empty(),
            "templates should be empty when inventory is full"
        );
        assert!(
            styles.is_empty(),
            "module styles should be empty when inventory is full — SSR already placed them"
        );
    }

    #[test]
    fn test_non_route_siblings_included_in_needed_components() {
        // Reproduces the commerce app layout: mp-app has both route children
        // (via outlet) AND non-route sibling components (mp-cart-panel, mp-footer).
        // All siblings must be in the needed set so their module CSS definitions
        // are emitted in <head> during SSR.
        let mut fragments = HashMap::new();

        // Entry → app-shell component
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("app-shell")],
            },
        );

        // app-shell has: navbar (component), route (outlet), cart-panel (component)
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::component("my-navbar"),
                    WebUIFragment::route_from(WebUiFragmentRoute {
                        path: "/about".into(),
                        fragment_id: "page-about".into(),
                        exact: true,
                        keep_alive: false,
                        ..Default::default()
                    }),
                    WebUIFragment::component("cart-panel"),
                ],
            },
        );

        // Leaf fragments
        fragments.insert(
            "my-navbar".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<nav/>")],
            },
        );
        fragments.insert(
            "page-about".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<h1>About</h1>")],
            },
        );
        fragments.insert(
            "cart-panel".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<aside>Cart</aside>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        for name in ["app-shell", "my-navbar", "page-about", "cart-panel"] {
            let comp = protocol.components.entry(name.to_string()).or_default();
            comp.template = format!("(function(){{/* {name} */}})();");
            comp.css = format!(".{name}{{display:block}}");
        }

        let (needed, _inv) =
            get_needed_components_for_request(&protocol, "index.html", "/about", "");

        assert!(
            needed.contains(&"app-shell".to_string()),
            "app-shell should be needed: {needed:?}"
        );
        assert!(
            needed.contains(&"my-navbar".to_string()),
            "my-navbar (non-route sibling) should be needed: {needed:?}"
        );
        assert!(
            needed.contains(&"page-about".to_string()),
            "page-about (active route) should be needed: {needed:?}"
        );
        assert!(
            needed.contains(&"cart-panel".to_string()),
            "cart-panel (non-route sibling) should be needed: {needed:?}"
        );
    }

    // ── RouteChainEntry.to_json + allowed_query tests ────────────────

    #[test]
    fn test_chain_entry_to_json_includes_allowed_query() {
        let entry = RouteChainEntry {
            component: "compose-page".into(),
            path: "compose".into(),
            params: HashMap::new(),
            exact: true,
            allowed_query: "action,to,subject".into(),
            keep_alive: false,
            cache_tags: Vec::new(),
            invalidates: Vec::new(),
            pending_component: String::new(),
            error_component: String::new(),
        };
        let json = entry.to_json();
        assert_eq!(json["allowedQuery"], "action,to,subject");
    }

    #[test]
    fn test_chain_entry_to_json_omits_empty_allowed_query() {
        let entry = RouteChainEntry {
            component: "home-page".into(),
            path: "/".into(),
            params: HashMap::new(),
            exact: true,
            allowed_query: String::new(),
            keep_alive: false,
            cache_tags: Vec::new(),
            invalidates: Vec::new(),
            pending_component: String::new(),
            error_component: String::new(),
        };
        let json = entry.to_json();
        assert!(
            json.get("allowedQuery").is_none(),
            "empty allowed_query should not appear in JSON"
        );
    }

    #[test]
    fn test_collect_route_chain_carries_allowed_query() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/".into(),
                    fragment_id: "app-shell".into(),
                    exact: false,
                    children: vec![WebUiFragmentRoute {
                        path: "compose".into(),
                        fragment_id: "compose-page".into(),
                        exact: true,
                        allowed_query: "action,to".into(),
                        keep_alive: false,
                        ..Default::default()
                    }],
                    keep_alive: false,
                    ..Default::default()
                })],
            },
        );
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<h1>App</h1>"), WebUIFragment::outlet()],
            },
        );
        fragments.insert(
            "compose-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Compose</p>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);

        let chain = collect_route_chain(&protocol, "index.html", "/compose");
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].component, "app-shell");
        assert!(chain[0].allowed_query.is_empty());
        assert_eq!(chain[1].component, "compose-page");
        assert_eq!(chain[1].allowed_query, "action,to");
    }

    #[test]
    fn test_render_component_templates_returns_template_and_css() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "settings-dialog".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div class='dialog'>Settings</div>")],
            },
        );
        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        let comp = protocol
            .components
            .entry("settings-dialog".to_string())
            .or_default();
        comp.template = "(function(){window.__webui_templates['settings-dialog']={h:'<div>Settings</div>'};})();".to_string();
        comp.css = ".dialog{position:fixed}".to_string();

        let result = render_component_templates(&protocol, &["settings-dialog"], "");
        let templates = result["templates"].as_array().expect("templates array");
        let styles = result["templateStyles"].as_array().expect("styles array");

        assert_eq!(templates.len(), 1);
        assert!(templates[0].as_str().unwrap().contains("settings-dialog"));
        assert_eq!(styles.len(), 1);
        assert!(styles[0]
            .as_str()
            .unwrap()
            .contains(".dialog{position:fixed}"));
    }

    #[test]
    fn test_render_component_templates_respects_inventory() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "my-dialog".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>Dialog</div>")],
            },
        );
        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        let comp = protocol
            .components
            .entry("my-dialog".to_string())
            .or_default();
        comp.template = "(function(){})();".to_string();
        comp.css = ".d{color:red}".to_string();

        // First call: no inventory → should return the component
        let result1 = render_component_templates(&protocol, &["my-dialog"], "");
        let inv = result1["inventory"].as_str().expect("inventory string");
        assert_eq!(result1["templates"].as_array().unwrap().len(), 1);

        // Second call with inventory → component already loaded, should skip
        let result2 = render_component_templates(&protocol, &["my-dialog"], inv);
        assert_eq!(result2["templates"].as_array().unwrap().len(), 0);
        assert_eq!(result2["templateStyles"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_render_component_templates_unknown_component_returns_empty() {
        let fragments = HashMap::new();
        let protocol = WebUIProtocol::with_tokens(fragments, Vec::new());

        let result = render_component_templates(&protocol, &["nonexistent-widget"], "");
        assert_eq!(result["templates"].as_array().unwrap().len(), 0);
        assert_eq!(result["templateStyles"].as_array().unwrap().len(), 0);
    }

    // ── resolve_tag_templates tests ──────────────────────────────────

    #[test]
    fn test_resolve_tag_templates_no_placeholders() {
        let tags = vec!["inbox".to_string(), "counts".to_string()];
        let params = HashMap::new();
        let resolved = resolve_tag_templates(&tags, &params);
        assert_eq!(resolved, vec!["inbox", "counts"]);
    }

    #[test]
    fn test_resolve_tag_templates_with_params() {
        let tags = vec!["thread:{threadId}".to_string(), "inbox".to_string()];
        let mut params = HashMap::new();
        params.insert("threadId".to_string(), "42".to_string());
        let resolved = resolve_tag_templates(&tags, &params);
        assert_eq!(resolved, vec!["thread:42", "inbox"]);
    }

    #[test]
    fn test_resolve_tag_templates_multiple_params() {
        let tags = vec!["folder:{folderId}:user:{userId}".to_string()];
        let mut params = HashMap::new();
        params.insert("folderId".to_string(), "drafts".to_string());
        params.insert("userId".to_string(), "abc".to_string());
        let resolved = resolve_tag_templates(&tags, &params);
        assert_eq!(resolved, vec!["folder:drafts:user:abc"]);
    }

    #[test]
    fn test_resolve_tag_templates_missing_param_left_unresolved() {
        let tags = vec!["thread:{threadId}".to_string()];
        let params = HashMap::new();
        let resolved = resolve_tag_templates(&tags, &params);
        assert_eq!(resolved, vec!["thread:{threadId}"]);
    }

    // ── Chain entry to_json with new fields ───────────────────────

    #[test]
    fn test_chain_entry_to_json_includes_new_fields() {
        let entry = RouteChainEntry {
            component: "mail-thread".into(),
            path: "email/:threadId".into(),
            params: {
                let mut p = HashMap::new();
                p.insert("threadId".to_string(), "42".to_string());
                p
            },
            exact: true,
            allowed_query: String::new(),
            keep_alive: false,
            cache_tags: vec!["thread:42".to_string()],
            invalidates: vec!["inbox".to_string(), "counts".to_string()],
            pending_component: "mail-skeleton".into(),
            error_component: "error-page".into(),
        };
        let json = entry.to_json();
        assert_eq!(json["pendingComponent"], "mail-skeleton");
        assert_eq!(json["errorComponent"], "error-page");
        let inv = json["invalidates"].as_array().unwrap();
        assert_eq!(inv.len(), 2);
        assert_eq!(inv[0], "inbox");
        assert_eq!(inv[1], "counts");
    }

    #[test]
    fn test_chain_entry_to_json_omits_empty_new_fields() {
        let entry = RouteChainEntry {
            component: "home-page".into(),
            path: "/".into(),
            params: HashMap::new(),
            exact: true,
            allowed_query: String::new(),
            keep_alive: false,
            cache_tags: Vec::new(),
            invalidates: Vec::new(),
            pending_component: String::new(),
            error_component: String::new(),
        };
        let json = entry.to_json();
        assert!(json.get("pendingComponent").is_none());
        assert!(json.get("errorComponent").is_none());
        assert!(json.get("invalidates").is_none());
    }

    // ── render_partial with cache tags ────────────────────────────

    #[test]
    fn test_render_partial_includes_resolved_cache_tags() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/".into(),
                    fragment_id: "app-shell".into(),
                    exact: false,
                    children: vec![WebUiFragmentRoute {
                        path: "email/:threadId".into(),
                        fragment_id: "mail-thread".into(),
                        exact: true,
                        cache_tags: vec!["thread:{threadId}".to_string(), "inbox".to_string()],
                        ..Default::default()
                    }],
                    cache_tags: vec!["folders".to_string()],
                    ..Default::default()
                })],
            },
        );
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<h1>App</h1>"), WebUIFragment::outlet()],
            },
        );
        fragments.insert(
            "mail-thread".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Thread</p>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);

        let partial = render_partial(&protocol, "index.html", "/email/42", "");
        let tags = partial["cacheTags"].as_array().unwrap();
        let tag_strings: Vec<&str> = tags.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(
            tag_strings.contains(&"folders"),
            "should contain parent tags"
        );
        assert!(
            tag_strings.contains(&"thread:42"),
            "should resolve threadId param to 42"
        );
        assert!(tag_strings.contains(&"inbox"), "should contain static tags");
    }

    // ── render_action_response tests ─────────────────────────────

    #[test]
    fn test_render_action_response_returns_resolved_invalidation_tags() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/".into(),
                    fragment_id: "app-shell".into(),
                    exact: false,
                    children: vec![WebUiFragmentRoute {
                        path: "compose".into(),
                        fragment_id: "compose-page".into(),
                        exact: true,
                        invalidates: vec![
                            "inbox".to_string(),
                            "sent".to_string(),
                            "counts".to_string(),
                        ],
                        ..Default::default()
                    }],
                    ..Default::default()
                })],
            },
        );
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<h1>App</h1>"), WebUIFragment::outlet()],
            },
        );
        fragments.insert(
            "compose-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Compose</p>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);

        let result = render_action_response(
            &protocol,
            serde_json::json!({"ok": true}),
            "index.html",
            "/compose",
        );

        let tags = result["invalidateTags"].as_array().unwrap();
        let tag_strings: Vec<&str> = tags.iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(tag_strings, vec!["inbox", "sent", "counts"]);
        assert_eq!(result["state"]["ok"], true);
    }

    #[test]
    fn test_render_action_response_resolves_param_placeholders() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/".into(),
                    fragment_id: "app-shell".into(),
                    exact: false,
                    children: vec![WebUiFragmentRoute {
                        path: "email/:threadId/reply".into(),
                        fragment_id: "reply-page".into(),
                        exact: true,
                        invalidates: vec!["thread:{threadId}".to_string(), "inbox".to_string()],
                        ..Default::default()
                    }],
                    ..Default::default()
                })],
            },
        );
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<h1>App</h1>"), WebUIFragment::outlet()],
            },
        );
        fragments.insert(
            "reply-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Reply</p>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);

        let result = render_action_response(
            &protocol,
            serde_json::json!({}),
            "index.html",
            "/email/42/reply",
        );

        let tags = result["invalidateTags"].as_array().unwrap();
        let tag_strings: Vec<&str> = tags.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(tag_strings.contains(&"thread:42"));
        assert!(tag_strings.contains(&"inbox"));
    }
}
