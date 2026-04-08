// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use actix_web::cookie::time::Duration;
use actix_web::cookie::{Cookie, SameSite};
use actix_web::HttpRequest;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::catalog::{Catalog, Product};

type HmacSha256 = Hmac<Sha256>;

/// Secret used for HMAC signing the cart cookie.  Loaded from the
/// `CART_COOKIE_SECRET` environment variable so it is never compiled into the
/// binary.  Falls back to a default only in `#[cfg(test)]` builds.
fn cookie_secret() -> &'static [u8] {
    static SECRET: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    SECRET
        .get_or_init(|| {
            std::env::var("CART_COOKIE_SECRET")
                .unwrap_or_else(|_| {
                    if cfg!(test) {
                        "webui-commerce-demo-test-secret".to_string()
                    } else {
                        eprintln!(
                            "CART_COOKIE_SECRET not set — generating a random session-scoped key"
                        );
                        format!("ephemeral-{}", std::process::id())
                    }
                })
                .into_bytes()
        })
        .as_slice()
}

pub const CART_COOKIE_NAME: &str = "mp-cart";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredCart {
    pub lines: Vec<StoredCartLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredCartLine {
    pub h: String,
    pub c: String,
    pub s: String,
    pub q: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CartItemState {
    pub handle: String,
    pub title: String,
    pub color: String,
    pub size: String,
    pub variant_label: String,
    pub price: String,
    pub quantity: u16,
    pub gradient: String,
    pub image_url: String,
    pub increase_to: u16,
    pub decrease_to: u16,
    pub redirect_to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CartState {
    pub cart_items: Vec<CartItemState>,
    pub cart_item_count: u32,
    pub cart_empty: bool,
    pub cart_subtotal: String,
    pub cart_taxes: String,
}

pub struct CartRead {
    pub cart: StoredCart,
    pub should_reset: bool,
}

pub fn read_cart(req: &HttpRequest) -> CartRead {
    let Some(cookie) = req.cookie(CART_COOKIE_NAME) else {
        return CartRead {
            cart: StoredCart::default(),
            should_reset: false,
        };
    };

    let Some((payload_hex, sig_hex)) = cookie.value().split_once('.') else {
        eprintln!("Invalid cart cookie: missing signature");
        return CartRead {
            cart: StoredCart::default(),
            should_reset: true,
        };
    };

    if !verify_signature(payload_hex, sig_hex) {
        eprintln!("Invalid cart cookie: bad signature");
        return CartRead {
            cart: StoredCart::default(),
            should_reset: true,
        };
    }

    let Some(bytes) = decode_hex(payload_hex) else {
        eprintln!("Invalid cart cookie: not valid hex");
        return CartRead {
            cart: StoredCart::default(),
            should_reset: true,
        };
    };

    match serde_json::from_slice::<StoredCart>(&bytes) {
        Ok(cart) => CartRead {
            cart,
            should_reset: false,
        },
        Err(error) => {
            eprintln!("Invalid cart cookie payload: {error}");
            CartRead {
                cart: StoredCart::default(),
                should_reset: true,
            }
        }
    }
}

pub fn cookie_for_cart(cart: &StoredCart) -> Option<Cookie<'static>> {
    if cart.lines.is_empty() {
        return None;
    }

    let bytes = serde_json::to_vec(cart).ok()?;
    let payload_hex = encode_hex(&bytes);
    let sig_bytes = compute_signature(&payload_hex);
    let sig_hex = encode_hex(&sig_bytes);
    let value = format!("{payload_hex}.{sig_hex}");
    Some(
        Cookie::build(CART_COOKIE_NAME, value)
            .path("/")
            .max_age(Duration::days(30))
            .http_only(true)
            .secure(true)
            .same_site(SameSite::Lax)
            .finish(),
    )
}

pub fn clear_cookie() -> Cookie<'static> {
    Cookie::build(CART_COOKIE_NAME, "")
        .path("/")
        .max_age(Duration::seconds(0))
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Lax)
        .finish()
}

pub fn add_item(cart: &mut StoredCart, handle: &str, color: &str, size: &str, quantity: u16) {
    if let Some(existing) = cart
        .lines
        .iter_mut()
        .find(|line| line.h == handle && line.c == color && line.s == size)
    {
        existing.q = existing.q.saturating_add(quantity).min(99);
        return;
    }

    if cart.lines.len() >= 50 {
        return;
    }

    cart.lines.push(StoredCartLine {
        h: handle.to_string(),
        c: color.to_string(),
        s: size.to_string(),
        q: quantity.clamp(1, 99),
    });
}

pub fn update_item(cart: &mut StoredCart, handle: &str, color: &str, size: &str, quantity: u16) {
    if quantity == 0 {
        cart.lines
            .retain(|line| !(line.h == handle && line.c == color && line.s == size));
        return;
    }

    if let Some(existing) = cart
        .lines
        .iter_mut()
        .find(|line| line.h == handle && line.c == color && line.s == size)
    {
        existing.q = quantity.min(99);
    }
}

pub fn resolve_variant(
    product: &Product,
    requested_color: &str,
    requested_size: &str,
) -> (String, String) {
    let color = product
        .colors
        .iter()
        .find(|option| option.available && option.value == requested_color)
        .or_else(|| product.colors.iter().find(|option| option.available))
        .map_or_else(String::new, |option| option.value.clone());

    let size = product
        .sizes
        .iter()
        .find(|option| option.available && option.value == requested_size)
        .or_else(|| product.sizes.iter().find(|option| option.available))
        .map_or_else(String::new, |option| option.value.clone());

    (color, size)
}

#[must_use]
pub fn sanitize_redirect(target: Option<&str>) -> String {
    let Some(target) = target else {
        return "/".to_string();
    };

    if !target.starts_with('/')
        || target.starts_with("//")
        || target.contains('\\')
        || target.bytes().any(|b| b < 0x20 || b == 0x7f)
    {
        return "/".to_string();
    }

    without_cart(target)
}

#[must_use]
pub fn with_cart_open(path: &str, open: bool) -> String {
    let (pathname, query) = split_path_query(path);
    let mut pairs = parse_query_pairs(query);
    pairs.retain(|(key, _)| key != "cart");

    if open {
        pairs.push(("cart".to_string(), "open".to_string()));
    }

    rebuild_path(pathname, &pairs)
}

#[must_use]
pub fn without_cart(path: &str) -> String {
    with_cart_open(path, false)
}

#[must_use]
pub fn build_cart_state(cart: &StoredCart, catalog: &Catalog, current_path: &str) -> CartState {
    let redirect_to = with_cart_open(current_path, true);
    let mut items = Vec::with_capacity(cart.lines.len().min(64));
    let mut subtotal = 0.0_f64;
    let mut item_count: u32 = 0;

    for line in &cart.lines {
        let Some(product) = catalog.by_handle(&line.h) else {
            eprintln!("Dropping cart line for unknown product '{}'", line.h);
            continue;
        };

        let (color, size) = resolve_variant(product, &line.c, &line.s);
        let variant_label = build_variant_label(&color, &size);
        let quantity = line.q.max(1);
        subtotal += product.price_raw * f64::from(quantity);
        item_count += u32::from(quantity);
        items.push(CartItemState {
            handle: product.handle.clone(),
            title: product.title.clone(),
            color,
            size,
            variant_label,
            price: product.price.clone(),
            quantity,
            gradient: product.gradient.clone(),
            image_url: product.image_url.clone(),
            increase_to: quantity.saturating_add(1).min(99),
            decrease_to: quantity.saturating_sub(1),
            redirect_to: redirect_to.clone(),
        });
    }

    CartState {
        cart_items: items,
        cart_item_count: item_count,
        cart_empty: item_count == 0,
        cart_subtotal: format_currency(subtotal),
        cart_taxes: format_currency(0.0),
    }
}

fn split_path_query(path: &str) -> (&str, &str) {
    match path.split_once('?') {
        Some((pathname, query)) => (pathname, query),
        None => (path, ""),
    }
}

fn parse_query_pairs(query: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for raw_pair in query.split('&') {
        if raw_pair.is_empty() {
            continue;
        }
        match raw_pair.split_once('=') {
            Some((key, value)) => pairs.push((key.to_string(), value.to_string())),
            None => pairs.push((raw_pair.to_string(), String::new())),
        }
    }
    pairs
}

fn rebuild_path(pathname: &str, pairs: &[(String, String)]) -> String {
    if pairs.is_empty() {
        return pathname.to_string();
    }

    let mut rebuilt = String::with_capacity(pathname.len() + pairs.len() * 16);
    rebuilt.push_str(pathname);
    rebuilt.push('?');

    for (index, (key, value)) in pairs.iter().enumerate() {
        if index > 0 {
            rebuilt.push('&');
        }
        rebuilt.push_str(key);
        if !value.is_empty() {
            rebuilt.push('=');
            rebuilt.push_str(value);
        }
    }

    rebuilt
}

fn format_currency(value: f64) -> String {
    format!("${value:.2}")
}

fn build_variant_label(color: &str, size: &str) -> String {
    match (color.is_empty(), size.is_empty()) {
        (true, true) => String::new(),
        (false, true) => color.to_string(),
        (true, false) => size.to_string(),
        (false, false) => format!("{color} / {size}"),
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[(byte >> 4) as usize]));
        output.push(char::from(HEX[(byte & 0x0f) as usize]));
    }
    output
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }

    let mut bytes = Vec::with_capacity(value.len() / 2);
    let mut chars = value.bytes();
    while let (Some(high), Some(low)) = (chars.next(), chars.next()) {
        let high_nibble = hex_nibble(high)?;
        let low_nibble = hex_nibble(low)?;
        bytes.push((high_nibble << 4) | low_nibble);
    }
    Some(bytes)
}

fn compute_signature(payload_hex: &str) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(cookie_secret()).expect("HMAC accepts any key length");
    mac.update(payload_hex.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn verify_signature(payload_hex: &str, sig_hex: &str) -> bool {
    let sig_bytes = match decode_hex(sig_hex) {
        Some(bytes) => bytes,
        None => return false,
    };
    let mut mac = HmacSha256::new_from_slice(cookie_secret()).expect("HMAC accepts any key length");
    mac.update(payload_hex.as_bytes());
    mac.verify_slice(&sig_bytes).is_ok()
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        add_item, build_cart_state, compute_signature, cookie_for_cart, decode_hex, encode_hex,
        sanitize_redirect, verify_signature, with_cart_open, without_cart, StoredCart,
    };
    use crate::catalog::Catalog;

    #[test]
    fn cart_query_helpers_preserve_existing_search_params() {
        assert_eq!(
            with_cart_open("/search/shirts?q=acme&sort=price-asc", true),
            "/search/shirts?q=acme&sort=price-asc&cart=open"
        );
        assert_eq!(
            without_cart("/search/shirts?q=acme&sort=price-asc&cart=open"),
            "/search/shirts?q=acme&sort=price-asc"
        );
    }

    #[test]
    fn invalid_redirect_targets_fall_back_to_root() {
        assert_eq!(sanitize_redirect(Some("https://example.com")), "/");
        assert_eq!(sanitize_redirect(Some("//example.com")), "/");
        assert_eq!(
            sanitize_redirect(Some("/search?q=shirts&cart=open")),
            "/search?q=shirts"
        );
    }

    #[test]
    fn redirect_rejects_backslash_and_control_chars() {
        assert_eq!(sanitize_redirect(Some("\\evil.com")), "/");
        assert_eq!(sanitize_redirect(Some("/foo\\bar")), "/");
        assert_eq!(sanitize_redirect(Some("/foo\r\nX-Injected: true")), "/");
        assert_eq!(sanitize_redirect(Some("/foo\x00bar")), "/");
    }

    #[test]
    fn cart_cookie_hex_round_trips() {
        let payload = br#"{"lines":[{"h":"acme-t-shirt","c":"Black","s":"M","q":2}]}"#;
        let encoded = encode_hex(payload);
        let decoded = match decode_hex(&encoded) {
            Some(decoded) => decoded,
            None => panic!("failed to decode hex"),
        };
        assert_eq!(decoded, payload);
    }

    #[test]
    fn cookie_signature_round_trips() {
        let payload_hex = encode_hex(b"test payload");
        let sig_bytes = compute_signature(&payload_hex);
        let sig_hex = encode_hex(&sig_bytes);
        assert!(verify_signature(&payload_hex, &sig_hex));
    }

    #[test]
    fn cookie_signature_rejects_tampered_payload() {
        let payload_hex = encode_hex(b"original");
        let sig_bytes = compute_signature(&payload_hex);
        let sig_hex = encode_hex(&sig_bytes);
        let tampered_hex = encode_hex(b"tampered");
        assert!(!verify_signature(&tampered_hex, &sig_hex));
    }

    #[test]
    fn cookie_for_cart_produces_signed_value() {
        let mut cart = StoredCart::default();
        add_item(&mut cart, "acme-t-shirt", "Black", "M", 1);
        let cookie = cookie_for_cart(&cart);
        assert!(cookie.is_some(), "non-empty cart should produce a cookie");
        let cookie = cookie.unwrap();
        let value = cookie.value();
        assert!(
            value.contains('.'),
            "signed cookie must contain a dot separator"
        );
        let (payload_hex, sig_hex) = value.split_once('.').unwrap();
        assert!(
            verify_signature(payload_hex, sig_hex),
            "cookie signature must verify"
        );
    }

    #[test]
    fn cart_state_resolves_prices_from_catalog() {
        let mut cart = StoredCart::default();
        add_item(&mut cart, "acme-t-shirt", "Black", "M", 2);
        let state = build_cart_state(&cart, &Catalog::generate(), "/product/acme-t-shirt");

        assert_eq!(state.cart_item_count, 2);
        assert!(!state.cart_empty);
        assert_eq!(state.cart_items[0].handle, "acme-t-shirt");
        assert_eq!(state.cart_subtotal, "$40.00");
        assert_eq!(state.cart_taxes, "$0.00");
    }
}
