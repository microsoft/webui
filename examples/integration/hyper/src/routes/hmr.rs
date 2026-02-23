use std::fs;
use std::time::SystemTime;

use bytes::Bytes;
use http_body_util::Full;
use hyper::{Response, StatusCode};

use crate::config::AppPaths;

/// HMR endpoint returning a version derived from the latest modification time
/// of the template or data file. The client polls this to detect changes.
pub fn handle_hmr(paths: &AppPaths) -> Response<Full<Bytes>> {
    let template_mtime = fs::metadata(&paths.template)
        .and_then(|m| m.modified())
        .ok();
    let data_mtime = fs::metadata(&paths.data).and_then(|m| m.modified()).ok();

    let latest: Option<SystemTime> = match (template_mtime, data_mtime) {
        (Some(t), Some(d)) => Some(if t > d { t } else { d }),
        (Some(t), None) => Some(t),
        (None, Some(d)) => Some(d),
        (None, None) => None,
    };

    let version_str = latest
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_millis().to_string())
        .unwrap_or_else(|| "0".to_string());

    Response::builder()
        .status(StatusCode::OK)
        .body(Full::new(Bytes::from(version_str)))
        .expect("valid response")
}
