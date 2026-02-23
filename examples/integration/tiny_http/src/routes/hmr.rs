use tiny_http::{Request, Response, StatusCode};

use crate::config::AppPaths;
use crate::hmr::hmr_version;

/// HMR route which will refresh when the template or data file changes.
/// Returns a version derived from the latest modification time of
/// template.html or data.json, so no shared counter is needed.
pub fn handle_hmr(request: Request, paths: &AppPaths) {
    let version_str = hmr_version(&paths.template, &paths.data);
    let response = Response::from_string(version_str).with_status_code(StatusCode(200));
    let _ = request.respond(response);
}
