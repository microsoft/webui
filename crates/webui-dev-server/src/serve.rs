// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Static-file actix handler for dev servers.
//!
//! Serves files from a single output directory with:
//!  - `basePath` segment-aware stripping (so `/webui-evil/x` does not
//!    match `/webui/`),
//!  - traversal-safe path resolution,
//!  - `<base href>`-aware redirects (`/foo` → `/foo/` for directory URLs),
//!  - automatic livereload script injection into HTML responses,
//!  - caller-controlled 404 strategy (plain text, custom file, etc.).
//!
//! webui-cli does NOT use this handler — its serve command renders
//! requests on the fly via the WebUI handler. webui-press uses it to
//! serve the prebuilt `out_dir`.

use std::path::{Path, PathBuf};

use actix_web::http::header::{
    CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, X_CONTENT_TYPE_OPTIONS,
};
use actix_web::http::StatusCode;
use actix_web::{HttpRequest, HttpResponse};

use crate::livereload::LiveReload;
use crate::path::{resolve_safe_path, strip_base_path};

/// What to serve when a requested file isn't found.
#[derive(Clone)]
pub enum NotFoundStrategy {
    /// Return a `text/plain; charset=utf-8` 404. The default.
    Plain,
    /// Serve `<root>/<file>` as the 404 body (typically `404.html`).
    /// Falls back to [`NotFoundStrategy::Plain`] if the file can't be
    /// read. Livereload script is injected when the file is HTML.
    File(PathBuf),
}

/// Configuration for [`serve_static_file`]. Cheap to clone — paths are
/// shared across requests.
#[derive(Clone)]
pub struct StaticServeConfig {
    /// Directory from which files are served.
    pub root: PathBuf,
    /// Application basePath. Use `"/"` when the app is hosted at root.
    /// Must be normalized via
    /// [`normalize_base_path`](crate::path::normalize_base_path).
    pub base_path: String,
    /// Live-reload broadcaster — its client script is injected into
    /// every HTML response served. Pass [`LiveReload::disabled`] (or
    /// any newly-constructed instance) to skip injection.
    pub livereload: LiveReload,
    /// What to serve on miss.
    pub not_found: NotFoundStrategy,
}

/// Serve `req` from `cfg`, returning the appropriate `HttpResponse`.
///
/// This function is not an actix handler itself — it's invoked by a
/// caller's `default_service` handler so the caller can attach app
/// state, middleware, and additional routes around it.
pub async fn serve_static_file(req: &HttpRequest, cfg: &StaticServeConfig) -> HttpResponse {
    let path = req.path();

    let remainder = match strip_base_path(path, &cfg.base_path) {
        Some(r) => r,
        None => {
            // Outside basePath: redirect "/" → basePath for browser
            // convenience, 404 everything else so missing-asset bugs
            // surface clearly.
            if path == "/" && cfg.base_path != "/" {
                return HttpResponse::TemporaryRedirect()
                    .insert_header((LOCATION, cfg.base_path.clone()))
                    .finish();
            }
            return not_found_response(cfg).await;
        }
    };

    // `/webui` (no trailing slash) → redirect to `/webui/` so relative
    // URLs resolve correctly in the browser.
    if cfg.base_path != "/" && format!("{path}/") == cfg.base_path {
        return HttpResponse::TemporaryRedirect()
            .insert_header((LOCATION, cfg.base_path.clone()))
            .finish();
    }

    let resolved = match resolve_safe_path(&cfg.root, remainder) {
        Some(p) => p,
        None => return not_found_response(cfg).await,
    };

    // If a directory was requested without a trailing slash, redirect.
    let final_path = if resolved.is_dir() {
        if !path.ends_with('/') {
            return HttpResponse::TemporaryRedirect()
                .insert_header((LOCATION, format!("{path}/")))
                .finish();
        }
        resolved.join("index.html")
    } else {
        resolved
    };

    match tokio::fs::read(&final_path).await {
        Ok(bytes) => serve_file_response(&cfg.livereload, &final_path, bytes),
        Err(_) => not_found_response(cfg).await,
    }
}

/// Build a 200 response for a successfully-read file, injecting the
/// livereload script into HTML payloads. Public so callers with custom
/// routing can serve files using the same headers/injection policy.
#[must_use]
pub fn serve_file_response(livereload: &LiveReload, path: &Path, bytes: Vec<u8>) -> HttpResponse {
    let mime = mime_guess::from_path(path)
        .first_or_octet_stream()
        .to_string();

    let is_html = mime.starts_with("text/html");
    let body = if is_html {
        // HTML must be valid UTF-8 (or ASCII). Reject invalid bytes
        // rather than silently corrupting them with replacement chars.
        match std::str::from_utf8(&bytes) {
            Ok(html) => livereload.inject(html).into_bytes(),
            Err(_) => bytes,
        }
    } else {
        bytes
    };

    let len = body.len();
    HttpResponse::Ok()
        .insert_header((CONTENT_TYPE, mime))
        .insert_header((CONTENT_LENGTH, len.to_string()))
        .insert_header((CACHE_CONTROL, "no-cache, no-store, must-revalidate"))
        .insert_header((X_CONTENT_TYPE_OPTIONS, "nosniff"))
        .body(body)
}

async fn not_found_response(cfg: &StaticServeConfig) -> HttpResponse {
    if let NotFoundStrategy::File(rel) = &cfg.not_found {
        let path = cfg.root.join(rel);
        if let Ok(bytes) = tokio::fs::read(&path).await {
            let mut resp = serve_file_response(&cfg.livereload, &path, bytes);
            *resp.status_mut() = StatusCode::NOT_FOUND;
            return resp;
        }
    }
    HttpResponse::NotFound()
        .content_type("text/plain; charset=utf-8")
        .body("404 Not Found")
}
