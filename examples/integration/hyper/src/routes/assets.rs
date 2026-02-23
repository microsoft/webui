use std::fs;
use std::path::Path;

use bytes::Bytes;
use http_body_util::Full;
use hyper::{Response, StatusCode};

use crate::config::AppPaths;

/// Serves files from the app's `assets/` directory.
///
/// Uses `fs::read` for binary-safe reads and maps the file extension
/// to a content type with a static string reference to avoid allocation.
pub fn handle_asset(path: &str, paths: &AppPaths) -> Response<Full<Bytes>> {
    // Strip the leading "/assets/" prefix and resolve against the app's assets dir
    let relative = path.strip_prefix("/assets/").unwrap_or(path);
    let asset_file_path = paths.assets_dir.join(relative);

    match fs::read(&asset_file_path) {
        Ok(contents) => {
            let content_type = content_type_for_path(&asset_file_path);
            Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", content_type)
                .body(Full::new(Bytes::from(contents)))
                .expect("valid response")
        }
        Err(err) => {
            eprintln!("Failed to read {}: {err}", asset_file_path.display());
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Full::new(Bytes::from_static(b"Not Found")))
                .expect("valid response")
        }
    }
}

/// Maps a file extension to a content-type string.
/// Returns a `&'static str` to avoid allocation.
fn content_type_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        _ => "application/octet-stream",
    }
}
