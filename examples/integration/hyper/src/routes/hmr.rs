use bytes::Bytes;
use http_body_util::Full;
use hyper::{Response, StatusCode};

use crate::config::AppPaths;
use crate::hmr::hmr_version;

/// HMR endpoint returning a version derived from the latest modification time
/// of the template or data file. The client polls this to detect changes.
pub fn handle_hmr(paths: &AppPaths) -> Response<Full<Bytes>> {
    let version_str = hmr_version(&paths.template, &paths.data);

    Response::builder()
        .status(StatusCode::OK)
        .body(Full::new(Bytes::from(version_str)))
        .expect("valid response")
}
