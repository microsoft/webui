// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use actix_web::web;
use serde::Deserialize;

use crate::cart::{self, CartState};
use crate::catalog::Catalog;

pub(crate) const STORE_NAME: &str = "Acme Store";

pub(crate) struct ShellContext<'a> {
    pub(crate) catalog: &'a Catalog,
    pub(crate) cart_state: &'a CartState,
    pub(crate) stable_path: String,
    pub(crate) cart_open: bool,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct PageQuery {
    pub(crate) q: Option<String>,
    pub(crate) sort: Option<String>,
    pub(crate) cart: Option<String>,
}

pub(crate) fn build_shell_context<'a>(
    catalog: &'a Catalog,
    request_path: &str,
    cart_state: &'a CartState,
) -> (ShellContext<'a>, PageQuery) {
    let query = parse_query(request_path);
    let cart_open = query.cart.as_deref() == Some("open");

    (
        ShellContext {
            catalog,
            cart_state,
            stable_path: cart::without_cart(request_path),
            cart_open,
        },
        query,
    )
}

fn parse_query(request_path: &str) -> PageQuery {
    web::Query::<PageQuery>::from_query(query_string(request_path))
        .map(|query| query.into_inner())
        .unwrap_or_default()
}

fn query_string(request_path: &str) -> &str {
    match request_path.split_once('?') {
        Some((_, query)) => query,
        None => "",
    }
}
