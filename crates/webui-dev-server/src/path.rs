// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! URL path utilities for dev servers.
//!
//! These helpers handle two concerns that every dev server needs:
//!  1. Strip an application `basePath` (e.g. `/webui/`) from incoming URLs
//!     on a segment boundary, so `/webui-evil/x` does NOT match `/webui/`.
//!  2. Resolve the remaining path to a file inside an output directory
//!     while rejecting traversal, percent-encoded `..`, backslashes, NUL,
//!     and absolute markers in any segment.

use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;

/// Normalize a basePath to either `/` or `/<segments>/` — exactly one
/// leading and one trailing slash. Used to make matching deterministic.
#[must_use]
pub fn normalize_base_path(raw: &str) -> String {
    let trimmed = raw.trim_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        format!("/{trimmed}/")
    }
}

/// Try to strip `base_path` from a URL path on a segment boundary.
///
/// Returns `Some(remainder)` if `path` is exactly `base_path` (with or
/// without trailing slash) or starts with `base_path`. Returns `None`
/// otherwise so the caller can decide between redirect and 404.
///
/// The remainder never has a leading `/`.
#[must_use]
pub fn strip_base_path<'a>(path: &'a str, base_path: &str) -> Option<&'a str> {
    if base_path == "/" {
        return Some(path.trim_start_matches('/'));
    }
    let without_trailing = base_path.trim_end_matches('/');
    if path == without_trailing || path == base_path {
        return Some("");
    }
    path.strip_prefix(base_path)
}

/// Resolve a URL path remainder (already base-stripped) to a filesystem
/// path inside `out_dir`. Returns `None` for any input that would escape
/// `out_dir` or that contains components forbidden for security reasons.
///
/// Rules:
///  - Percent-decode each segment.
///  - Reject any segment equal to `..`, `.`, empty (consecutive `//`),
///    or containing `\` / NUL.
///  - Reject Windows drive prefixes / absolute markers in any segment.
///  - If the original path ends with `/` or is empty, append `index.html`.
#[must_use]
pub fn resolve_safe_path(out_dir: &Path, remainder: &str) -> Option<PathBuf> {
    let trailing_slash = remainder.is_empty() || remainder.ends_with('/');
    let trimmed = remainder.trim_matches('/');

    let mut buf = out_dir.to_path_buf();
    if !trimmed.is_empty() {
        for raw_segment in trimmed.split('/') {
            let decoded = percent_decode_str(raw_segment).decode_utf8().ok()?;
            let s = decoded.as_ref();
            if s.is_empty() || s == "." || s == ".." {
                return None;
            }
            if s.contains('\\') || s.contains('\0') {
                return None;
            }
            if s.starts_with('/') {
                return None;
            }
            // Reject Windows drive-letter prefixes (e.g. "C:") to avoid
            // `PathBuf::push` jumping to an absolute path on Windows.
            if has_drive_letter_prefix(s) {
                return None;
            }
            buf.push(s);
        }
    }

    if trailing_slash {
        buf.push("index.html");
    }
    Some(buf)
}

/// Returns true if `s` begins with a Windows drive-letter prefix
/// (e.g. `C:` or `c:foo`). `PathBuf::push` treats such segments as
/// absolute, so we reject them defensively even on non-Windows hosts.
fn has_drive_letter_prefix(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic()) && matches!(chars.next(), Some(':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_base_path_handles_root() {
        assert_eq!(normalize_base_path("/"), "/");
        assert_eq!(normalize_base_path(""), "/");
    }

    #[test]
    fn normalize_base_path_adds_slashes() {
        assert_eq!(normalize_base_path("webui"), "/webui/");
        assert_eq!(normalize_base_path("/webui"), "/webui/");
        assert_eq!(normalize_base_path("/webui/"), "/webui/");
        assert_eq!(normalize_base_path("a/b"), "/a/b/");
    }

    #[test]
    fn strip_base_path_root() {
        assert_eq!(strip_base_path("/", "/"), Some(""));
        assert_eq!(strip_base_path("/foo.css", "/"), Some("foo.css"));
        assert_eq!(strip_base_path("/guide/intro/", "/"), Some("guide/intro/"));
    }

    #[test]
    fn strip_base_path_segment_boundary() {
        assert_eq!(strip_base_path("/webui/", "/webui/"), Some(""));
        assert_eq!(strip_base_path("/webui", "/webui/"), Some(""));
        assert_eq!(strip_base_path("/webui/guide/", "/webui/"), Some("guide/"));
        assert_eq!(
            strip_base_path("/webui/foo.css", "/webui/"),
            Some("foo.css")
        );
    }

    #[test]
    fn strip_base_path_rejects_similar_prefix() {
        assert_eq!(strip_base_path("/webui-evil/x", "/webui/"), None);
        assert_eq!(strip_base_path("/webuixyz", "/webui/"), None);
        assert_eq!(strip_base_path("/", "/webui/"), None);
        assert_eq!(strip_base_path("/other/foo", "/webui/"), None);
    }

    #[test]
    fn resolve_safe_path_basic() {
        let out = Path::new("/tmp/dist");
        assert_eq!(
            resolve_safe_path(out, "").as_deref(),
            Some(Path::new("/tmp/dist/index.html"))
        );
        assert_eq!(
            resolve_safe_path(out, "foo.css").as_deref(),
            Some(Path::new("/tmp/dist/foo.css"))
        );
        assert_eq!(
            resolve_safe_path(out, "guide/intro/").as_deref(),
            Some(Path::new("/tmp/dist/guide/intro/index.html"))
        );
    }

    #[test]
    fn resolve_safe_path_rejects_traversal() {
        let out = Path::new("/tmp/dist");
        assert_eq!(resolve_safe_path(out, "../etc/passwd"), None);
        assert_eq!(resolve_safe_path(out, "foo/../../bar"), None);
        assert_eq!(resolve_safe_path(out, "%2e%2e/etc"), None);
        assert_eq!(resolve_safe_path(out, "foo/%2e%2e/bar"), None);
        assert_eq!(resolve_safe_path(out, "foo//bar"), None);
        assert_eq!(resolve_safe_path(out, "foo\\bar"), None);
        assert_eq!(resolve_safe_path(out, "./foo"), None);
    }

    #[test]
    fn resolve_safe_path_decodes_legal_percent() {
        let out = Path::new("/tmp/dist");
        assert_eq!(
            resolve_safe_path(out, "my%20file.html").as_deref(),
            Some(Path::new("/tmp/dist/my file.html"))
        );
    }

    #[test]
    fn resolve_safe_path_rejects_drive_letters() {
        let out = Path::new("/tmp/dist");
        assert_eq!(resolve_safe_path(out, "C:/Windows/System32"), None);
        assert_eq!(resolve_safe_path(out, "c:foo"), None);
        assert_eq!(resolve_safe_path(out, "foo/C:/bar"), None);
        // Decoded form should also be rejected.
        assert_eq!(resolve_safe_path(out, "C%3A/win"), None);
    }
}
