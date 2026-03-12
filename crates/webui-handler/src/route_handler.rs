//! Route component inventory management for incremental route rendering.

use std::collections::HashSet;

use webui_protocol::{web_ui_fragment, WebUIProtocol};

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
    let mut chars = hex.chars();
    while let (Some(hi), Some(lo)) = (chars.next(), chars.next()) {
        if let Ok(byte) = u8::from_str_radix(&format!("{hi}{lo}"), 16) {
            bytes.push(byte);
        }
    }
    bytes
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
    let inv = parse_inventory(inventory_hex);

    // Walk the fragment graph from entry_id
    let mut visited = HashSet::new();
    let mut stack = vec![entry_id.to_string()];

    while let Some(frag_id) = stack.pop() {
        if frag_id.is_empty() || !visited.insert(frag_id.clone()) {
            continue;
        }
        if let Some(frag_list) = protocol.fragments.get(&frag_id) {
            for frag in &frag_list.fragments {
                match frag.fragment.as_ref() {
                    Some(web_ui_fragment::Fragment::Component(c)) => {
                        stack.push(c.fragment_id.clone());
                    }
                    Some(web_ui_fragment::Fragment::ForLoop(fl)) => {
                        stack.push(fl.fragment_id.clone());
                    }
                    Some(web_ui_fragment::Fragment::IfCond(ic)) => {
                        stack.push(ic.fragment_id.clone());
                    }
                    Some(web_ui_fragment::Fragment::Attribute(attr))
                        if !attr.template.is_empty() =>
                    {
                        stack.push(attr.template.clone());
                    }
                    _ => {}
                }
            }
        }
    }

    // Filter to component names (contain hyphen, not for-/if-/attr- prefixed)
    let needed: Vec<String> = visited
        .iter()
        .filter(|id| {
            id.contains('-')
                && !id.starts_with("for-")
                && !id.starts_with("if-")
                && !id.starts_with("attr-")
                && !has_component(&inv, id)
        })
        .cloned()
        .collect();

    // Build updated inventory
    let mut updated_inv = inv;
    updated_inv.resize(32, 0);
    for name in &needed {
        let bit = component_bit_position(name);
        let byte_idx = (bit / 8) as usize;
        let bit_idx = bit % 8;
        if byte_idx < updated_inv.len() {
            updated_inv[byte_idx] |= 1 << bit_idx;
        }
    }

    (needed, encode_inventory(&updated_inv))
}

/// Get the f-template HTML strings needed for a route that the client
/// doesn't already have.
///
/// Walks the fragment graph from `entry_id`, identifies needed components
/// (not in the client's inventory), and returns their f-template HTML
/// from the protocol's `component_templates` map.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use webui_protocol::{FragmentList, RouteRecord, WebUIFragment};

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

        // First call with empty inventory
        let (_needed, inv_hex) = get_needed_components(&protocol, "app-shell", "");
        // Second call with populated inventory — should return empty
        let (needed2, _) = get_needed_components(&protocol, "app-shell", &inv_hex);
        assert!(needed2.is_empty());
    }
}
