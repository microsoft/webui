// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use actix_web::{web, HttpRequest, HttpResponse};
use awc::Client;
use std::collections::HashMap;

// Maximum response body size (16 MB) — generous for HTML/JSON/assets.
const MAX_BODY_SIZE: usize = 16 * 1024 * 1024;

/// Shared state mapping app slugs to their internal ports.
pub(crate) struct ProxyState {
    pub routes: HashMap<String, u16>,
}

/// Reverse proxy handler: strips the `/{slug}/` prefix and forwards
/// the request to the app's internal port.
pub(crate) async fn proxy_handler(
    req: HttpRequest,
    body: web::Bytes,
    path: web::Path<(String, String)>,
    state: web::Data<ProxyState>,
    client: web::Data<Client>,
) -> HttpResponse {
    let (slug, tail) = path.into_inner();

    let port = match state.routes.get(&slug) {
        Some(p) => *p,
        None => return HttpResponse::NotFound().body(format!("Unknown app: {slug}")),
    };

    let forward_path = if tail.is_empty() {
        "/".to_string()
    } else {
        format!("/{tail}")
    };

    // Preserve query string
    let query = req
        .uri()
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();

    let target_url = format!("http://127.0.0.1:{port}{forward_path}{query}");

    // Build the forwarded request
    let mut forwarded = client.request(req.method().clone(), &target_url);

    // Copy relevant headers (skip Host — we set our own)
    for (name, value) in req.headers() {
        if name != "host" {
            forwarded = forwarded.insert_header((name.clone(), value.clone()));
        }
    }

    // Add forwarding headers
    if let Some(peer) = req.peer_addr() {
        forwarded = forwarded.insert_header(("X-Forwarded-For", peer.ip().to_string()));
    }
    if let Some(host) = req.headers().get("host") {
        forwarded =
            forwarded.insert_header(("X-Forwarded-Host", host.to_str().unwrap_or_default()));
    }
    forwarded = forwarded.insert_header(("X-Forwarded-Proto", req.connection_info().scheme()));

    // Send the request
    let response = match forwarded.send_body(body).await {
        Ok(r) => r,
        Err(e) => {
            log::error!("Proxy error for {slug} → {target_url}: {e}");
            return HttpResponse::BadGateway().body(format!("App '{slug}' is unavailable: {e}"));
        }
    };

    // Build the response back to the client
    let status = response.status();
    let mut builder = HttpResponse::build(status);

    // Copy response headers
    for (name, value) in response.headers() {
        // awc auto-decompresses response bodies (gzip, br, deflate),
        // so the original content-encoding and content-length no longer
        // apply to the bytes we forward.
        let skip =
            name == "transfer-encoding" || name == "content-encoding" || name == "content-length";
        if !skip {
            builder.insert_header((name.clone(), value.clone()));
        }
    }

    // Set SAMEORIGIN for all proxied responses
    builder.insert_header(("X-Frame-Options", "SAMEORIGIN"));

    // Stream the response body — no rewriting needed since the app
    // servers emit relative asset paths via --base-path.
    let mut response = response;
    match response.body().limit(MAX_BODY_SIZE).await {
        Ok(bytes) => builder.body(bytes),
        Err(e) => {
            log::error!("Failed to read response body from {slug}: {e}");
            HttpResponse::BadGateway().body("Failed to read response from app")
        }
    }
}

/// Handler for the root of an app slug (e.g., `/calculator` without trailing slash).
/// Redirects to `/{slug}/` so relative paths resolve correctly.
pub(crate) async fn slug_redirect(
    path: web::Path<String>,
    state: web::Data<ProxyState>,
) -> HttpResponse {
    let slug = path.into_inner();
    if state.routes.contains_key(&slug) {
        HttpResponse::MovedPermanently()
            .insert_header(("Location", format!("/{slug}/")))
            .finish()
    } else {
        HttpResponse::NotFound().body(format!("Unknown app: {slug}"))
    }
}
