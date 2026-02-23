use std::fs;
use std::time::SystemTime;

use tiny_http::{Request, Response, StatusCode};

// HMR route which will refresh when the template or data file changes
// Returns a version derived from the latest modification time of
// template.html or data.json, so no shared counter is needed.
pub fn handle_hmr(request: Request) {
    let template_mtime = fs::metadata("../../app/hello-world/templates/index.html")
        .and_then(|m| m.modified())
        .ok();
    let data_mtime =
	fs::metadata("../../app/hello-world/data/state.json").and_then(|m| m.modified()).ok();

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

    let response = Response::from_string(version_str).with_status_code(StatusCode(200));
    let _ = request.respond(response);
}
