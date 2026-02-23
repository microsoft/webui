use bytes::Bytes;
use http_body_util::Full;
use hyper::{Response, StatusCode};

/// Returns a 404 Not Found response.
pub fn handle_not_found() -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Full::new(Bytes::from_static(b"Not Found")))
        .expect("valid response")
}
