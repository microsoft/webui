use std::fs;

use bytes::Bytes;
use http_body_util::Full;
use hyper::{Response, StatusCode};

/// Serves `dist/index.html` with `text/html` content type.
pub fn handle_index() -> Response<Full<Bytes>> {
    match fs::read("dist/index.html") {
        Ok(contents) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/html; charset=utf-8")
            .body(Full::new(Bytes::from(contents)))
            .expect("valid response"),
        Err(err) => {
            eprintln!("Failed to read index.html: {err}");
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Full::new(Bytes::from_static(b"Internal Server Error")))
                .expect("valid response")
        }
    }
}
