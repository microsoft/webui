// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use serde_json::Value;
use webui_handler::route_matcher::RouteMatch;

use crate::cart::CartState;
use crate::catalog::Catalog;

use super::catalog_pages;
use super::content_pages;
use super::context::build_shell_context;
use super::route_keys;

pub(crate) fn build_route_state(
    catalog: &Catalog,
    route_match: &RouteMatch,
    request_path: &str,
    cart_state: &CartState,
) -> Option<Value> {
    let (context, query) = build_shell_context(catalog, request_path, cart_state);
    let query_text = query.q.as_deref().unwrap_or_default();
    let requested_sort = query.sort.as_deref();

    match route_match.route_key.as_str() {
        route_keys::HOME => Some(catalog_pages::home_state(&context)),
        route_keys::SEARCH => Some(catalog_pages::search_state(
            &context,
            query_text,
            requested_sort,
        )),
        route_keys::CATEGORY => {
            let category = route_match.params.get("category")?;
            catalog_pages::category_state(&context, category, query_text, requested_sort)
        }
        route_keys::PRODUCT => {
            let handle = route_match.params.get("handle")?;
            catalog_pages::product_state(&context, handle)
        }
        _ => content_pages::static_page_state(&context, route_match.route_key.as_str()),
    }
}
