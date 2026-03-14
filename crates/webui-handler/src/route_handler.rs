//! Route component inventory management for incremental route rendering.
//!
//! These helpers walk the normal render fragment graph. The request-aware path is
//! route-aware but state-agnostic: it follows the active route chain for the
//! current request path, while conservatively traversing `if`, `for`, and
//! attribute-template edges without evaluating runtime state.

use crate::route_matcher;
use std::collections::HashSet;
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
}

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

        let matched_route_key =
            request_path.and_then(|path| find_best_route_match(&frag_list.fragments, path));

        for frag in &frag_list.fragments {
            match frag.fragment.as_ref() {
                Some(Fragment::Component(component)) => {
                    stack.push(QueuedFragment {
                        id: component.fragment_id.clone(),
                        inventoryable: true,
                    });
                }
                Some(Fragment::ForLoop(for_loop)) => {
                    stack.push(QueuedFragment {
                        id: for_loop.fragment_id.clone(),
                        inventoryable: false,
                    });
                }
                Some(Fragment::IfCond(if_cond)) => {
                    stack.push(QueuedFragment {
                        id: if_cond.fragment_id.clone(),
                        inventoryable: false,
                    });
                }
                Some(Fragment::Attribute(attr)) if !attr.template.is_empty() => {
                    stack.push(QueuedFragment {
                        id: attr.template.clone(),
                        inventoryable: false,
                    });
                }
                Some(Fragment::Route(route_frag)) => {
                    let is_selected = matched_route_key
                        .as_ref()
                        .is_some_and(|best| best == route_fragment_key(route_frag));
                    if is_selected && !route_frag.fragment_id.is_empty() {
                        stack.push(QueuedFragment {
                            id: route_frag.fragment_id.clone(),
                            inventoryable: protocol
                                .component_templates
                                .contains_key(&route_frag.fragment_id),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    component_ids
}

fn find_best_route_match(fragments: &[WebUIFragment], request_path: &str) -> Option<String> {
    let mut best_key: Option<String> = None;
    let mut best_specificity: usize = 0;

    for item in fragments {
        if let Some(Fragment::Route(route_frag)) = item.fragment.as_ref() {
            if let Some(m) =
                route_matcher::match_single_route(&route_frag.path, request_path, route_frag.exact)
            {
                if best_key.is_none() || m.specificity > best_specificity {
                    best_specificity = m.specificity;
                    best_key = Some(route_fragment_key(route_frag).to_string());
                }
            }
        }
    }

    best_key
}

fn route_fragment_key(route_frag: &WebUIFragmentRoute) -> &str {
    if route_frag.name.is_empty() {
        route_frag.fragment_id.as_str()
    } else {
        route_frag.name.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use webui_protocol::{FragmentList, RouteRecord, WebUIFragment, WebUiFragmentRoute};

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

        let routes = HashMap::new();
        let protocol = WebUIProtocol::with_routes(fragments, Vec::new(), routes);
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

        let routes = HashMap::new();
        let mut protocol = WebUIProtocol::with_routes(fragments, Vec::new(), routes);
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

        let routes: HashMap<String, RouteRecord> = HashMap::new();
        let protocol = WebUIProtocol::with_routes(fragments, Vec::new(), routes);

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

        let routes: HashMap<String, RouteRecord> = HashMap::new();
        let protocol = WebUIProtocol::with_routes(fragments, Vec::new(), routes);
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
                        name: "search".into(),
                        ..Default::default()
                    }),
                    WebUIFragment::route_from(WebUiFragmentRoute {
                        path: "/product/:handle".into(),
                        fragment_id: "mp-product-page".into(),
                        exact: true,
                        name: "product".into(),
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

        let routes = HashMap::new();
        let mut protocol = WebUIProtocol::with_routes(fragments, Vec::new(), routes);
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
                    name: "account".into(),
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
                        name: "profile".into(),
                        ..Default::default()
                    }),
                    WebUIFragment::route_from(WebUiFragmentRoute {
                        path: "/account/orders/:id".into(),
                        fragment_id: "mp-order-page".into(),
                        exact: true,
                        name: "order".into(),
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

        let routes = HashMap::new();
        let mut protocol = WebUIProtocol::with_routes(fragments, Vec::new(), routes);
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
                    name: "search".into(),
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

        let routes = HashMap::new();
        let mut protocol = WebUIProtocol::with_routes(fragments, Vec::new(), routes);
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
                    name: "search".into(),
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

        let routes = HashMap::new();
        let mut protocol = WebUIProtocol::with_routes(fragments, Vec::new(), routes);
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
                    name: "search".into(),
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

        let routes = HashMap::new();
        let mut protocol = WebUIProtocol::with_routes(fragments, Vec::new(), routes);
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
