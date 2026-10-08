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

use std::io::{Error, ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use actix_web::http::header::{
    CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, X_CONTENT_TYPE_OPTIONS,
};
use actix_web::http::StatusCode;
use actix_web::{HttpRequest, HttpResponse};

use crate::livereload::LiveReload;
use crate::path::{resolve_safe_path, strip_base_path};
use crate::secure_file::{OpenedNode, SecureRoot};

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

/// Configuration for [`serve_static_file`].
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

/// Prevalidated static-file configuration for repeated requests.
///
/// Prefer this configuration with [`serve_prepared_static_file`] in
/// long-running servers. It anchors the serving root once and shares that
/// capability across blocking file tasks.
#[derive(Clone)]
pub struct PreparedStaticServeConfig {
    root: SecureRoot,
    base_path: String,
    livereload: LiveReload,
    not_found: NotFoundStrategy,
}

impl PreparedStaticServeConfig {
    /// Create a prepared static-file configuration rooted at `root`.
    ///
    /// The root must exist so it can be anchored once during setup. Every
    /// request is confined beneath this trusted root before its file is read.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when `root` cannot be canonicalized or opened.
    #[must_use = "the configuration or its I/O error must be handled"]
    pub fn new(
        root: PathBuf,
        base_path: String,
        livereload: LiveReload,
        not_found: NotFoundStrategy,
    ) -> std::io::Result<Self> {
        Ok(Self {
            root: SecureRoot::new(root)?,
            base_path,
            livereload,
            not_found,
        })
    }
}

impl TryFrom<StaticServeConfig> for PreparedStaticServeConfig {
    type Error = std::io::Error;

    fn try_from(config: StaticServeConfig) -> Result<Self, Self::Error> {
        Self::new(
            config.root,
            config.base_path,
            config.livereload,
            config.not_found,
        )
    }
}

/// Serve `req` from `cfg`, returning the appropriate `HttpResponse`.
///
/// This function is not an actix handler itself — it's invoked by a
/// caller's `default_service` handler so the caller can attach app
/// state, middleware, and additional routes around it.
///
/// This compatibility entry point prepares the serving root for each call.
/// Long-running servers should prepare once and use
/// [`serve_prepared_static_file`] instead.
pub async fn serve_static_file(req: &HttpRequest, cfg: &StaticServeConfig) -> HttpResponse {
    let config = cfg.clone();
    match tokio::task::spawn_blocking(move || PreparedStaticServeConfig::try_from(config)).await {
        Ok(Ok(config)) => serve_prepared_static_file(req, Arc::new(config)).await,
        Ok(Err(error)) => static_config_error_response(error),
        Err(error) => file_task_error_response(error),
    }
}

/// Serve `req` from a prevalidated static-file configuration.
///
/// Share one configuration across requests with [`Arc`] to keep blocking
/// filesystem work batched without rebuilding the root capability.
pub async fn serve_prepared_static_file(
    req: &HttpRequest,
    cfg: Arc<PreparedStaticServeConfig>,
) -> HttpResponse {
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
            return not_found_response(Arc::clone(&cfg)).await;
        }
    };

    // `/webui` (no trailing slash) → redirect to `/webui/` so relative
    // URLs resolve correctly in the browser.
    if cfg.base_path != "/" && format!("{path}/") == cfg.base_path {
        return HttpResponse::TemporaryRedirect()
            .insert_header((LOCATION, cfg.base_path.clone()))
            .finish();
    }

    let resolved = match resolve_safe_path(cfg.root.path(), remainder) {
        Some(p) => p,
        None => return not_found_response(Arc::clone(&cfg)).await,
    };

    let detect_directory = !path.ends_with('/');
    match run_file_load(resolved, detect_directory, Arc::clone(&cfg)).await {
        Ok(FileLoad::Found {
            path,
            bytes,
            status,
        }) => file_response(&cfg.livereload, &path, bytes, status),
        Ok(FileLoad::Directory) => {
            if detect_directory {
                HttpResponse::TemporaryRedirect()
                    .insert_header((LOCATION, format!("{path}/")))
                    .finish()
            } else {
                file_load_invariant_response()
            }
        }
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
    cfg: Arc<PreparedStaticServeConfig>,
) -> Result<FileLoad, tokio::task::JoinError> {
    tokio::task::spawn_blocking(move || load_file(path, detect_directory, &cfg)).await
}

async fn run_fallback_load(
    cfg: Arc<PreparedStaticServeConfig>,
) -> Result<FileLoad, tokio::task::JoinError> {
    tokio::task::spawn_blocking(move || load_fallback(&cfg)).await
}

fn load_file(path: PathBuf, detect_directory: bool, cfg: &PreparedStaticServeConfig) -> FileLoad {
    match load_opened_file(path, detect_directory, StatusCode::OK, cfg) {
        Ok(file) => file,
        Err(_) => load_fallback(cfg),
    }
}

fn load_fallback(cfg: &PreparedStaticServeConfig) -> FileLoad {
    let NotFoundStrategy::File(relative) = &cfg.not_found else {
        return FileLoad::NotFound;
    };
    match load_opened_file(cfg.root.join(relative), false, StatusCode::NOT_FOUND, cfg) {
        Ok(file) => file,
        Err(_) => FileLoad::NotFound,
    }
}

fn load_opened_file(
    path: PathBuf,
    detect_directory: bool,
    status: StatusCode,
    cfg: &PreparedStaticServeConfig,
) -> std::io::Result<FileLoad> {
    let OpenedNode::File {
        path,
        mut file,
        length,
    } = cfg.root.open(path, detect_directory)?
    else {
        return if detect_directory {
            Ok(FileLoad::Directory)
        } else {
            Err(Error::new(
                ErrorKind::InvalidInput,
                "cannot serve a directory as a file",
            ))
        };
    };
    let capacity = usize::try_from(length).map_err(|_| {
        Error::new(
            ErrorKind::InvalidData,
            "file length exceeds the addressable buffer size",
        )
    })?;
    let mut bytes = Vec::with_capacity(capacity);
    file.read_to_end(&mut bytes)?;
    Ok(FileLoad::Found {
        path,
        bytes,
        status,
    })
}

async fn not_found_response(cfg: Arc<PreparedStaticServeConfig>) -> HttpResponse {
    if matches!(&cfg.not_found, NotFoundStrategy::Plain) {
        return plain_not_found_response();
    }
    match run_fallback_load(Arc::clone(&cfg)).await {
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
    eprintln!("static-file task failed: {error}");
    HttpResponse::InternalServerError()
        .content_type("text/plain; charset=utf-8")
        .body("Internal Server Error")
}

#[cold]
#[inline(never)]
fn static_config_error_response(error: std::io::Error) -> HttpResponse {
    eprintln!("static-file setup failed: {error}");
    HttpResponse::InternalServerError()
        .content_type("text/plain; charset=utf-8")
        .body("Internal Server Error")
}

#[cold]
#[inline(never)]
fn file_load_invariant_response() -> HttpResponse {
    eprintln!("static-file loader reported a directory without a redirect target");
    HttpResponse::InternalServerError()
        .content_type("text/plain; charset=utf-8")
        .body("Internal Server Error")
}

#[cfg(test)]
#[path = "serve_tests.rs"]
mod tests;
