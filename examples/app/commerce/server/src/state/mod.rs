// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

mod catalog_pages;
mod content_pages;
mod context;
mod route_keys;
mod routes;
mod shell;

pub(crate) use routes::{build_route_state, RouteStateRequest};
pub(crate) use shell::cart_state_payload;

#[cfg(test)]
mod tests {
    use super::{build_route_state, cart_state_payload, RouteStateRequest};
    use crate::cart::{add_item, build_cart_state, StoredCart};
    use crate::catalog::Catalog;

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
        let params =
            std::collections::HashMap::from([("handle".to_string(), "acme-t-shirt".to_string())]);

        let state = match build_route_state(&RouteStateRequest {
            catalog: &catalog,
            route_path: "/product/acme-t-shirt",
            params: &params,
            request_path: "/product/acme-t-shirt",
            cart_state: &cart_state,
            is_partial: false,
        }) {
            Some((state, _)) => state,
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
        let params = std::collections::HashMap::new();

        let state = match build_route_state(&RouteStateRequest {
            catalog: &catalog,
            route_path: "/about",
            params: &params,
            request_path: "/about",
            cart_state: &cart_state,
            is_partial: false,
        }) {
            Some((state, _)) => state,
            None => panic!("expected about content state"),
        };

        assert_eq!(state["page"], "about");
        assert_eq!(state["showCatalogNav"], "");
        assert_eq!(state["shellClass"], "default-shell");
    }

    #[test]
    fn category_state_includes_current_category_label() {
        let catalog = Catalog::generate();
        let cart_state = build_cart_state(&StoredCart::default(), &catalog, "/search/footware");
        let params =
            std::collections::HashMap::from([("category".to_string(), "footware".to_string())]);

        let state = match build_route_state(&RouteStateRequest {
            catalog: &catalog,
            route_path: "/search/footware",
            params: &params,
            request_path: "/search/footware",
            cart_state: &cart_state,
            is_partial: false,
        }) {
            Some((state, _)) => state,
            None => panic!("expected category state"),
        };

        assert_eq!(state["currentCategoryLabel"], "Footware");
    }

    #[test]
    fn partial_response_excludes_shell_state() {
        let catalog = Catalog::generate();
        let cart_state = build_cart_state(&StoredCart::default(), &catalog, "/search/shirts");
        let params =
            std::collections::HashMap::from([("category".to_string(), "shirts".to_string())]);

        let state = match build_route_state(&RouteStateRequest {
            catalog: &catalog,
            route_path: "/search/shirts",
            params: &params,
            request_path: "/search/shirts",
            cart_state: &cart_state,
            is_partial: true,
        }) {
            Some((state, _)) => state,
            None => panic!("expected partial state"),
        };

        // Page-specific state should be present
        assert!(state.get("products").is_some());
        assert!(state.get("categories").is_some());
        assert!(state.get("sortOptions").is_some());

        // Shell state should be absent
        assert!(state.get("storeName").is_none());
        assert!(state.get("cartItems").is_none());
        assert!(state.get("cartItemCount").is_none());
        assert!(state.get("navCategories").is_none());
        assert!(state.get("page").is_none());
        assert!(state.get("shellClass").is_none());
    }

    #[test]
    fn home_state_returns_image_preloads() {
        let catalog = Catalog::generate();
        let cart_state = build_cart_state(&StoredCart::default(), &catalog, "/");
        let params = std::collections::HashMap::new();

        let (_, image_preloads) = match build_route_state(&RouteStateRequest {
            catalog: &catalog,
            route_path: "/",
            params: &params,
            request_path: "/",
            cart_state: &cart_state,
            is_partial: false,
        }) {
            Some(result) => result,
            None => panic!("expected home state"),
        };

        assert!(
            !image_preloads.is_empty(),
            "home page should have image preloads"
        );
        assert!(
            image_preloads[0].contains("/_image/"),
            "image preloads should reference image proxy"
        );
    }

    #[test]
    fn home_partial_returns_no_image_preloads() {
        let catalog = Catalog::generate();
        let cart_state = build_cart_state(&StoredCart::default(), &catalog, "/");
        let params = std::collections::HashMap::new();

        let (state, image_preloads) = match build_route_state(&RouteStateRequest {
            catalog: &catalog,
            route_path: "/",
            params: &params,
            request_path: "/",
            cart_state: &cart_state,
            is_partial: true,
        }) {
            Some(result) => result,
            None => panic!("expected home partial state"),
        };

        assert!(
            image_preloads.is_empty(),
            "partial response should not include image preloads"
        );
        assert!(
            state.get("head_end").is_none(),
            "partial response should not include head_end"
        );
    }

    #[test]
    fn product_state_returns_image_preloads() {
        let catalog = Catalog::generate();
        let cart_state =
            build_cart_state(&StoredCart::default(), &catalog, "/product/acme-t-shirt");
        let params =
            std::collections::HashMap::from([("handle".to_string(), "acme-t-shirt".to_string())]);

        let (_, image_preloads) = match build_route_state(&RouteStateRequest {
            catalog: &catalog,
            route_path: "/product/acme-t-shirt",
            params: &params,
            request_path: "/product/acme-t-shirt",
            cart_state: &cart_state,
            is_partial: false,
        }) {
            Some(result) => result,
            None => panic!("expected product state"),
        };

        assert_eq!(
            image_preloads.len(),
            1,
            "product page should preload hero image"
        );
        assert!(image_preloads[0].contains("/_image/"));
    }

    #[test]
    fn search_state_returns_image_preloads() {
        let catalog = Catalog::generate();
        let cart_state = build_cart_state(&StoredCart::default(), &catalog, "/search");
        let params = std::collections::HashMap::new();

        let (_, image_preloads) = match build_route_state(&RouteStateRequest {
            catalog: &catalog,
            route_path: "/search",
            params: &params,
            request_path: "/search",
            cart_state: &cart_state,
            is_partial: false,
        }) {
            Some(result) => result,
            None => panic!("expected search state"),
        };

        assert!(
            !image_preloads.is_empty(),
            "search page should preload first product image"
        );
    }

    #[test]
    fn category_state_returns_image_preloads() {
        let catalog = Catalog::generate();
        let cart_state = build_cart_state(&StoredCart::default(), &catalog, "/search/shirts");
        let params =
            std::collections::HashMap::from([("category".to_string(), "shirts".to_string())]);

        let (_, image_preloads) = match build_route_state(&RouteStateRequest {
            catalog: &catalog,
            route_path: "/search/shirts",
            params: &params,
            request_path: "/search/shirts",
            cart_state: &cart_state,
            is_partial: false,
        }) {
            Some(result) => result,
            None => panic!("expected category state"),
        };

        assert!(
            !image_preloads.is_empty(),
            "category page should preload first product image"
        );
    }
}
