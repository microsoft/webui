// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

mod catalog_pages;
mod content_pages;
mod context;
mod route_keys;
mod routes;
mod shell;

pub(crate) use routes::build_route_state;
pub(crate) use shell::cart_state_payload;

#[cfg(test)]
mod tests {
    use super::{build_route_state, cart_state_payload};
    use crate::cart::{add_item, build_cart_state, StoredCart};
    use crate::catalog::Catalog;
    use crate::state::route_keys;
    use webui_handler::route_matcher::RouteMatch;

    #[test]
    fn cart_payload_sets_shell_links() {
        let payload = cart_state_payload(
            &crate::cart::CartState {
                cart_items: Vec::new(),
                cart_item_count: 0,
                cart_empty: true,
                cart_subtotal: "$0.00".to_string(),
                cart_taxes: "$0.00".to_string(),
            },
            "/search/shirts?q=acme",
            true,
        );

        assert_eq!(payload["cartHref"], "/search/shirts?q=acme&cart=open");
        assert_eq!(payload["cartCloseHref"], "/search/shirts?q=acme");
    }

    #[test]
    fn product_state_includes_default_variant_fields() {
        let catalog = Catalog::generate();
        let mut cart = StoredCart::default();
        add_item(&mut cart, "acme-t-shirt", "Black", "M", 1);
        let cart_state = build_cart_state(&cart, &catalog, "/product/acme-t-shirt");

        let state = match build_route_state(
            &catalog,
            &RouteMatch {
                route_key: route_keys::PRODUCT.to_string(),
                params: std::collections::HashMap::from([(
                    "handle".to_string(),
                    "acme-t-shirt".to_string(),
                )]),
                specificity: 1,
            },
            "/product/acme-t-shirt",
            &cart_state,
        ) {
            Some(state) => state,
            None => panic!("expected product state"),
        };

        assert_eq!(state["page"], "product");
        assert!(state.get("defaultColor").is_some());
        assert!(state.get("defaultSize").is_some());
    }

    #[test]
    fn content_routes_only_need_shell_state() {
        let catalog = Catalog::generate();
        let cart_state = build_cart_state(&StoredCart::default(), &catalog, "/about");

        let state = match build_route_state(
            &catalog,
            &RouteMatch {
                route_key: route_keys::ABOUT.to_string(),
                params: std::collections::HashMap::new(),
                specificity: 1,
            },
            "/about",
            &cart_state,
        ) {
            Some(state) => state,
            None => panic!("expected about content state"),
        };

        assert_eq!(state["page"], "about");
        assert_eq!(state["showCatalogNav"], "");
        assert_eq!(state["shellClass"], "default-shell");
        assert!(state.get("pageContent").is_none());
        assert!(state.get("pageKey").is_none());
        assert!(state.get("pageTitle").is_none());
    }

    #[test]
    fn category_state_includes_current_category_label() {
        let catalog = Catalog::generate();
        let cart_state = build_cart_state(&StoredCart::default(), &catalog, "/search/footware");

        let state = match build_route_state(
            &catalog,
            &RouteMatch {
                route_key: route_keys::CATEGORY.to_string(),
                params: std::collections::HashMap::from([(
                    "category".to_string(),
                    "footware".to_string(),
                )]),
                specificity: 1,
            },
            "/search/footware",
            &cart_state,
        ) {
            Some(state) => state,
            None => panic!("expected category state"),
        };

        assert_eq!(state["currentCategoryLabel"], "Footware");
    }
}
