// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use actix_web::dev::Payload;
use actix_web::{web, Either, FromRequest, HttpRequest};
use serde::Deserialize;
use std::future::{ready, Ready};

use crate::cart::{self, CartRead};
use crate::frontend::{request_path, route_path, wants_json};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CartMutationInput {
    pub(crate) handle: String,
    pub(crate) color: Option<String>,
    pub(crate) size: Option<String>,
    pub(crate) quantity: Option<u16>,
    pub(crate) redirect_to: Option<String>,
    pub(crate) open_cart: Option<bool>,
}

pub(crate) type CartMutationPayload =
    Either<web::Form<CartMutationInput>, web::Json<CartMutationInput>>;

pub(crate) struct RequestContext {
    route_path: String,
    request_path: String,
    wants_json: bool,
    inventory_hex: String,
    cart_read: CartRead,
}

impl RequestContext {
    #[must_use]
    pub(crate) fn asset_path(&self) -> Option<&str> {
        let relative = self.route_path.trim_start_matches('/');
        if relative.is_empty() {
            None
        } else {
            Some(relative)
        }
    }

    #[must_use]
    pub(crate) fn route_path(&self) -> &str {
        &self.route_path
    }

    #[must_use]
    pub(crate) fn request_path(&self) -> &str {
        &self.request_path
    }

    #[must_use]
    pub(crate) fn wants_json(&self) -> bool {
        self.wants_json
    }

    #[must_use]
    pub(crate) fn inventory_hex(&self) -> &str {
        &self.inventory_hex
    }

    #[must_use]
    pub(crate) fn cart_read(&self) -> &CartRead {
        &self.cart_read
    }

    #[must_use]
    pub(crate) fn into_cart_read(self) -> CartRead {
        self.cart_read
    }
}

impl FromRequest for RequestContext {
    type Error = actix_web::Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        ready(Ok(Self {
            route_path: route_path(req).to_string(),
            request_path: request_path(req),
            wants_json: wants_json(req),
            inventory_hex: req
                .headers()
                .get("x-webui-inventory")
                .and_then(|value| value.to_str().ok())
                .map_or_else(String::new, ToString::to_string),
            cart_read: cart::read_cart(req),
        }))
    }
}
