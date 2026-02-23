use std::fs;
use tiny_http::{Header, Request, Response, StatusCode};

use crate::config::AppPaths;

// Asset file route handler that serves files from the app's assets/ directory
pub fn handle_asset(request: Request, paths: &AppPaths) {
    let url_path = request.url();
    let path = url_path.split('?').next().unwrap_or("/");
    // Strip the leading "/assets/" prefix and resolve against the app's assets dir
    let relative = path.strip_prefix("/assets/").unwrap_or(path);
    let asset_file_path = paths.assets_dir.join(relative);

    let body = match fs::read_to_string(&asset_file_path) {
        Ok(contents) => contents,
        Err(err) => {
            eprintln!("Failed to read {}: {err}", asset_file_path.display());
            let response = Response::from_string("Not Found").with_status_code(StatusCode(404));
            let _ = request.respond(response);
            return;
        }
    };

    let mut response = Response::from_string(body).with_status_code(StatusCode(200));

    // Determine content type based on file extension
    let content_type = match asset_file_path.extension().and_then(|ext| ext.to_str()) {
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        _ => "text/plain; charset=utf-8",
    };

    if let Ok(header) = Header::from_bytes(b"Content-Type" as &[u8], content_type.as_bytes()) {
        response.add_header(header);
    }

    let _ = request.respond(response);
}
