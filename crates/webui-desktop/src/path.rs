// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;

/// Resolve a custom-protocol path into a filesystem path under `root`.
///
/// The input must be a URL path or path remainder. It is percent-decoded one
/// segment at a time and rejects traversal, empty segments, current-directory
/// segments, backslashes, NUL bytes, absolute markers, and Windows drive
/// prefixes. The returned path is not canonicalized because the target may not
/// exist; callers that read from disk must canonicalize the final file and
/// verify it still starts with the canonical asset root.
#[must_use]
pub fn resolve_safe_path(root: &Path, request_path: &str) -> Option<PathBuf> {
    let path = request_path
        .split_once('?')
        .map_or(request_path, |(p, _)| p);
    let trailing_slash = path.is_empty() || path.ends_with('/');
    let trimmed = path.trim_matches('/');

    let mut resolved = root.to_path_buf();
    if !trimmed.is_empty() {
        for raw_segment in trimmed.split('/') {
            let decoded = percent_decode_str(raw_segment).decode_utf8().ok()?;
            let segment = decoded.as_ref();
            if !is_safe_segment(segment) {
                return None;
            }
            resolved.push(segment);
        }
    }

    if trailing_slash {
        resolved.push("index.html");
    }
    Some(resolved)
}

#[must_use]
fn is_safe_segment(segment: &str) -> bool {
    if segment.is_empty() || segment == "." || segment == ".." {
        return false;
    }
    if segment.contains('\\') || segment.contains('\0') || segment.starts_with('/') {
        return false;
    }
    !has_drive_letter_prefix(segment)
}

fn has_drive_letter_prefix(segment: &str) -> bool {
    let mut chars = segment.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic()) && matches!(chars.next(), Some(':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_basic_paths() {
        let root = Path::new("/tmp/app");
        assert_eq!(
            resolve_safe_path(root, "/index.css").as_deref(),
            Some(Path::new("/tmp/app/index.css"))
        );
        assert_eq!(
            resolve_safe_path(root, "/images/logo%20mark.svg").as_deref(),
            Some(Path::new("/tmp/app/images/logo mark.svg"))
        );
    }

    #[test]
    fn appends_index_for_directory_paths() {
        let root = Path::new("/tmp/app");
        assert_eq!(
            resolve_safe_path(root, "/").as_deref(),
            Some(Path::new("/tmp/app/index.html"))
        );
        assert_eq!(
            resolve_safe_path(root, "/docs/").as_deref(),
            Some(Path::new("/tmp/app/docs/index.html"))
        );
    }

    #[test]
    fn rejects_traversal_and_absolute_segments() {
        let root = Path::new("/tmp/app");
        assert_eq!(resolve_safe_path(root, "../secrets"), None);
        assert_eq!(resolve_safe_path(root, "/a/%2e%2e/b"), None);
        assert_eq!(resolve_safe_path(root, "/a//b"), None);
        assert_eq!(resolve_safe_path(root, "/a/./b"), None);
        assert_eq!(resolve_safe_path(root, "/a\\b"), None);
        assert_eq!(resolve_safe_path(root, "/C:/Windows"), None);
        assert_eq!(resolve_safe_path(root, "/C%3A/Windows"), None);
    }
}
