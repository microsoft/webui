// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use serde_json::Value;
use std::collections::HashMap;

use crate::cart::CartState;
use crate::catalog::Catalog;

use super::catalog_pages;
use super::content_pages;
use super::context::build_shell_context;
use super::route_keys;

/// Parameters for building route state.
pub(crate) struct RouteStateRequest<'a> {
    pub(crate) catalog: &'a Catalog,
    pub(crate) route_path: &'a str,
    pub(crate) params: &'a HashMap<String, String>,
    pub(crate) request_path: &'a str,
    pub(crate) cart_state: &'a CartState,
    /// When true, omit shell state (storeName, cart, nav) from the response.
    /// Used for JSON partial responses where the client already has the shell.
    pub(crate) is_partial: bool,
}

/// Determine the route key from the URL path.
fn route_key_from_path(route_path: &str) -> &str {
    let path = route_path.trim_start_matches('/');
    if path.is_empty() {
        return route_keys::HOME;
    }
    let first_segment = path.split('/').next().unwrap_or("");
    match first_segment {
        "search" => {
            // /search vs /search/:category
            if path.contains('/') && path != "search" {
                route_keys::CATEGORY
            } else {
                route_keys::SEARCH
            }
        }
        "product" => route_keys::PRODUCT,
        "about" => route_keys::ABOUT,
        "terms-conditions" => route_keys::TERMS,
        "shipping-return-policy" => route_keys::SHIPPING,
        "privacy-policy" => route_keys::PRIVACY,
        "frequently-asked-questions" => route_keys::FAQ,
        _ => route_keys::HOME,
    }
}

/// Build page state and return SSR image preload URLs for the initial document.
pub(crate) fn build_route_state(req: &RouteStateRequest<'_>) -> Option<(Value, Vec<String>)> {
    let (context, query) = build_shell_context(req.catalog, req.request_path, req.cart_state);
    let query_text = query.q.as_deref().unwrap_or_default();
    let requested_sort = query.sort.as_deref();
    let key = route_key_from_path(req.route_path);

    match key {
        route_keys::HOME => Some(catalog_pages::home_state(&context, req.is_partial)),
        route_keys::SEARCH => Some(catalog_pages::search_state(
            &context,
            query_text,
            requested_sort,
            req.is_partial,
        )),
        route_keys::CATEGORY => {
            let category = req.params.get("category")?;
            catalog_pages::category_state(
                &context,
                category,
                query_text,
                requested_sort,
                req.is_partial,
            )
        }
        route_keys::PRODUCT => {
            let handle = req.params.get("handle")?;
            catalog_pages::product_state(&context, handle, req.is_partial)
        }
        _ => {
            content_pages::static_page_state(&context, key, req.is_partial).map(|v| (v, Vec::new()))
        }
    }
}
