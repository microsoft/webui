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

    let redirect = if path.ends_with('/') {
        None
    } else {
        Some(format!("{path}/"))
    };
    let fallback = not_found_path(cfg);
    match run_file_load(resolved, redirect.is_some(), fallback).await {
        Ok(FileLoad::Found {
            path,
            bytes,
            status,
        }) => file_response(&cfg.livereload, &path, bytes, status),
        Ok(FileLoad::Directory) => match redirect {
            Some(location) => HttpResponse::TemporaryRedirect()
                .insert_header((LOCATION, location))
                .finish(),
            None => file_load_invariant_response(),
        },
        Ok(FileLoad::NotFound) => plain_not_found_response(),
        Err(error) => file_task_error_response(error),
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

enum FileLoad {
    Found {
        path: PathBuf,
        bytes: Vec<u8>,
        status: StatusCode,
    },
    Directory,
    NotFound,
}

async fn run_file_load(
    path: PathBuf,
    detect_directory: bool,
    fallback: Option<PathBuf>,
) -> Result<FileLoad, tokio::task::JoinError> {
    tokio::task::spawn_blocking(move || load_file(path, detect_directory, fallback)).await
}

async fn run_fallback_load(fallback: PathBuf) -> Result<FileLoad, tokio::task::JoinError> {
    tokio::task::spawn_blocking(move || load_fallback(Some(fallback))).await
}

fn load_file(path: PathBuf, detect_directory: bool, fallback: Option<PathBuf>) -> FileLoad {
    match std::fs::read(&path) {
        Ok(bytes) => FileLoad::Found {
            path,
            bytes,
            status: StatusCode::OK,
        },
        Err(_) if detect_directory && path.is_dir() => FileLoad::Directory,
        Err(_) => load_fallback(fallback),
    }
}

fn load_fallback(path: Option<PathBuf>) -> FileLoad {
    let Some(path) = path else {
        return FileLoad::NotFound;
    };
    match std::fs::read(&path) {
        Ok(bytes) => FileLoad::Found {
            path,
            bytes,
            status: StatusCode::NOT_FOUND,
        },
        Err(_) => FileLoad::NotFound,
    }
}

fn not_found_path(cfg: &StaticServeConfig) -> Option<PathBuf> {
    match &cfg.not_found {
        NotFoundStrategy::Plain => None,
        NotFoundStrategy::File(relative) => Some(cfg.root.join(relative)),
    }
}

async fn not_found_response(cfg: &StaticServeConfig) -> HttpResponse {
    let Some(fallback) = not_found_path(cfg) else {
        return plain_not_found_response();
    };
    match run_fallback_load(fallback).await {
        Ok(FileLoad::Found {
            path,
            bytes,
            status,
        }) => file_response(&cfg.livereload, &path, bytes, status),
        Ok(_) => plain_not_found_response(),
        Err(error) => file_task_error_response(error),
    }
}

fn file_response(
    livereload: &LiveReload,
    path: &Path,
    bytes: Vec<u8>,
    status: StatusCode,
) -> HttpResponse {
    let mut response = serve_file_response(livereload, path, bytes);
    *response.status_mut() = status;
    response
}

fn plain_not_found_response() -> HttpResponse {
    HttpResponse::NotFound()
        .content_type("text/plain; charset=utf-8")
        .body("404 Not Found")
}

#[cold]
#[inline(never)]
fn file_task_error_response(error: tokio::task::JoinError) -> HttpResponse {
    log::error!("Static-file blocking task failed: {error}");
    HttpResponse::InternalServerError()
        .content_type("text/plain; charset=utf-8")
        .body("Internal Server Error")
}

#[cold]
#[inline(never)]
fn file_load_invariant_response() -> HttpResponse {
    log::error!("Static-file loader reported a directory without a redirect target");
    HttpResponse::InternalServerError()
        .content_type("text/plain; charset=utf-8")
        .body("Internal Server Error")
}

#[cfg(test)]
mod tests {
    use actix_web::body::to_bytes;
    use actix_web::http::header::LOCATION;
    use actix_web::test::TestRequest;

    use super::*;

    fn config(root: PathBuf, not_found: NotFoundStrategy) -> StaticServeConfig {
        StaticServeConfig {
            root,
            base_path: "/".to_owned(),
            livereload: LiveReload::new("/__test/livereload"),
            not_found,
        }
    }

    #[actix_web::test]
    async fn serves_file_bytes() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let bytes = b"console.log('served');";
        std::fs::write(directory.path().join("app.js"), bytes)?;
        let request = TestRequest::with_uri("/app.js").to_http_request();

        let response = serve_static_file(
            &request,
            &config(directory.path().to_path_buf(), NotFoundStrategy::Plain),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(to_bytes(response.into_body()).await?.as_ref(), bytes);
        Ok(())
    }

    #[actix_web::test]
    async fn redirects_directory_without_trailing_slash() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        std::fs::create_dir(directory.path().join("guide"))?;
        let request = TestRequest::with_uri("/guide").to_http_request();

        let response = serve_static_file(
            &request,
            &config(directory.path().to_path_buf(), NotFoundStrategy::Plain),
        )
        .await;

        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(response.headers().get(LOCATION), Some(&"/guide/".parse()?));
        Ok(())
    }

    #[actix_web::test]
    async fn serves_custom_not_found_file() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let bytes = b"custom not found";
        std::fs::write(directory.path().join("404.txt"), bytes)?;
        let request = TestRequest::with_uri("/missing.txt").to_http_request();

        let response = serve_static_file(
            &request,
            &config(
                directory.path().to_path_buf(),
                NotFoundStrategy::File(PathBuf::from("404.txt")),
            ),
        )
        .await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(to_bytes(response.into_body()).await?.as_ref(), bytes);
        Ok(())
    }
}
