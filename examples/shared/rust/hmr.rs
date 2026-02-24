use std::fs;
use std::path::Path;
use std::time::SystemTime;

/// Compute an HMR version string from the latest modification time
/// of the template and data files. Returns a millisecond timestamp
/// or `"0"` if neither file has a retrievable modification time.
pub fn hmr_version(template: &Path, data: &Path) -> String {
    let template_mtime = fs::metadata(template).and_then(|m| m.modified()).ok();
    let data_mtime = fs::metadata(data).and_then(|m| m.modified()).ok();

    let latest: Option<SystemTime> = match (template_mtime, data_mtime) {
        (Some(t), Some(d)) => Some(if t > d { t } else { d }),
        (Some(t), None) => Some(t),
        (None, Some(d)) => Some(d),
        (None, None) => None,
    };

    latest
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_millis().to_string())
        .unwrap_or_else(|| "0".to_string())
}
