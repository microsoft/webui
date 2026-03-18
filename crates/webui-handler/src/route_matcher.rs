// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Iterative path template matcher for server-side route resolution.
//!
//! Matches request paths against route path templates without regex.
//! Supports `:param`, `:param?` (optional), and `*splat` segments.

use std::collections::HashMap;

/// Result of matching a request path against a route path template.
#[derive(Debug, Clone)]
pub struct RouteMatch {
    /// The matched route name (or fragment ID).
    pub route_key: String,
    /// Bound parameter values from the path.
    pub params: HashMap<String, String>,
    /// How many literal (non-param) segments matched exactly.
    pub specificity: usize,
    /// How many request path segments were consumed by the match.
    /// Used by nested routes to compute the child route base.
    pub consumed_segments: usize,
}

/// A single parsed segment from a path template.
#[derive(Debug, Clone)]
enum SegmentPattern {
    /// Literal segment (e.g., "users").
    Literal(String),
    /// Named parameter (e.g., `:id`).
    Param(String),
    /// Optional named parameter (e.g., `:id?`).
    OptionalParam(String),
    /// Splat / catch-all (e.g., `*rest` or `*`).
    Splat(String),
}

/// Parse a path template string into segment patterns.
fn parse_template(template: &str) -> Vec<SegmentPattern> {
    let mut segments = Vec::new();

    for part in template.split('/') {
        if part.is_empty() {
            continue;
        }

        if let Some(rest) = part.strip_prefix('*') {
            let name = if rest.is_empty() {
                "rest".to_string()
            } else {
                rest.to_string()
            };
            segments.push(SegmentPattern::Splat(name));
        } else if let Some(rest) = part.strip_prefix(':') {
            if let Some(name) = rest.strip_suffix('?') {
                segments.push(SegmentPattern::OptionalParam(name.to_string()));
            } else {
                segments.push(SegmentPattern::Param(rest.to_string()));
            }
        } else {
            segments.push(SegmentPattern::Literal(part.to_string()));
        }
    }

    segments
}

/// Split a request path into segments, filtering empty parts.
fn split_path(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

/// Sanitize a route parameter value to prevent path traversal.
/// Removes `..` path traversal sequences and null bytes.
fn sanitize_param(value: &str) -> String {
    value.replace('\0', "").replace("..", "")
}

/// Try matching a request path against pre-parsed route template patterns.
///
/// Returns `Some(RouteMatch)` if the path matches, `None` otherwise.
fn try_match(
    route_key: &str,
    patterns: &[SegmentPattern],
    request_segments: &[&str],
    exact: bool,
) -> Option<RouteMatch> {
    let mut params = HashMap::new();
    let mut specificity: usize = 0;
    let mut req_idx = 0;
    let mut pat_idx = 0;

    while pat_idx < patterns.len() {
        match &patterns[pat_idx] {
            SegmentPattern::Literal(expected) => {
                if req_idx >= request_segments.len() {
                    return None;
                }
                if request_segments[req_idx] != expected.as_str() {
                    return None;
                }
                specificity += 1;
                req_idx += 1;
            }
            SegmentPattern::Param(name) => {
                if req_idx >= request_segments.len() {
                    return None;
                }
                params.insert(name.clone(), sanitize_param(request_segments[req_idx]));
                req_idx += 1;
            }
            SegmentPattern::OptionalParam(name) => {
                if req_idx < request_segments.len() {
                    params.insert(name.clone(), sanitize_param(request_segments[req_idx]));
                    req_idx += 1;
                }
                // Optional — ok to skip
            }
            SegmentPattern::Splat(name) => {
                // Splat consumes all remaining segments
                let remaining: Vec<&str> = request_segments[req_idx..].to_vec();
                params.insert(name.clone(), sanitize_param(&remaining.join("/")));
                req_idx = request_segments.len();
            }
        }
        pat_idx += 1;
    }

    // If exact matching required, all request segments must be consumed
    if exact && req_idx < request_segments.len() {
        return None;
    }

    // All pattern segments must be consumed (non-optional ones already checked)
    // But remaining patterns must all be optional
    while pat_idx < patterns.len() {
        match &patterns[pat_idx] {
            SegmentPattern::OptionalParam(_) | SegmentPattern::Splat(_) => {}
            _ => return None,
        }
        pat_idx += 1;
    }

    Some(RouteMatch {
        route_key: route_key.to_string(),
        params,
        specificity,
        consumed_segments: req_idx,
    })
}

/// Match a single route template against a request path.
///
/// Returns `Some(RouteMatch)` if the path matches, `None` otherwise.
/// Used by the SSR route pruning to check individual `<f-route>` elements.
pub fn match_single_route(template: &str, request_path: &str, exact: bool) -> Option<RouteMatch> {
    let request_segments = split_path(request_path);
    let patterns = parse_template(template);
    try_match("", &patterns, &request_segments, exact)
}

/// Check whether a route path is relative (does NOT start with `/`).
pub fn is_relative_path(path: &str) -> bool {
    !path.is_empty() && !path.starts_with('/')
}

/// Resolve a route path against a base path.
///
/// - Relative paths (`topics/:id` or `./topics/:id`) are prepended with the base.
/// - Absolute paths (`/topics/:id`) are returned unchanged.
///
/// `"topics/:id"` + `"/sections/1"` → `"/sections/1/topics/:id"`
/// `"./topics/:id"` + `"/sections/1"` → `"/sections/1/topics/:id"`
/// `"/topics/:id"` + `"/sections/1"` → `"/topics/:id"`
pub fn resolve_route_path(path: &str, route_base: &str) -> String {
    // An empty path means "match at the parent level" — resolve to base.
    if path.is_empty() {
        return route_base.to_string();
    }

    if !is_relative_path(path) {
        return path.to_string();
    }

    // Strip leading "./" if present
    let relative = path.strip_prefix("./").unwrap_or(path);
    if relative.is_empty() {
        return route_base.to_string();
    }

    let mut resolved = String::with_capacity(route_base.len() + relative.len() + 1);
    resolved.push_str(route_base);
    if !route_base.ends_with('/') {
        resolved.push('/');
    }
    resolved.push_str(relative);
    resolved
}

/// Compute the new route base from a matched route's consumed request segments.
///
/// Given request path `/sections/1/topics/react` and consumed=2,
/// returns `/sections/1`.
pub fn compute_route_base(request_path: &str, consumed_segments: usize) -> String {
    let parts = split_path(request_path);
    if consumed_segments == 0 || parts.is_empty() {
        return "/".to_string();
    }

    let n = consumed_segments.min(parts.len());
    let mut base = String::with_capacity(request_path.len());
    for part in &parts[..n] {
        base.push('/');
        base.push_str(part);
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_match_single_route_exact() {
        let m = match_single_route("/contacts", "/contacts", true).unwrap();
        assert!(m.params.is_empty());
    }

    #[test]
    fn test_match_single_route_param() {
        let m = match_single_route("/contacts/:id", "/contacts/42", true).unwrap();
        assert_eq!(m.params["id"], "42");
    }

    #[test]
    fn test_match_single_route_no_match_extra() {
        assert!(match_single_route("/contacts", "/contacts/123", true).is_none());
    }

    #[test]
    fn test_match_single_route_multiple_params() {
        let m = match_single_route("/profile/:id/view/:section", "/profile/123/view/bio", true)
            .unwrap();
        assert_eq!(m.params["id"], "123");
        assert_eq!(m.params["section"], "bio");
    }

    #[test]
    fn test_match_single_route_splat() {
        let m = match_single_route("/files/*path", "/files/docs/readme.md", false).unwrap();
        assert_eq!(m.params["path"], "docs/readme.md");
    }

    #[test]
    fn test_match_single_route_optional_param_present() {
        let m = match_single_route("/search/:query?", "/search/hello", true).unwrap();
        assert_eq!(m.params["query"], "hello");
    }

    #[test]
    fn test_match_single_route_optional_param_absent() {
        let m = match_single_route("/search/:query?", "/search", true).unwrap();
        assert!(!m.params.contains_key("query"));
    }

    #[test]
    fn test_parse_template_segments() {
        let segs = parse_template("/users/:id/posts/:postId");
        assert_eq!(segs.len(), 4);
        assert!(matches!(&segs[0], SegmentPattern::Literal(s) if s == "users"));
        assert!(matches!(&segs[1], SegmentPattern::Param(s) if s == "id"));
        assert!(matches!(&segs[2], SegmentPattern::Literal(s) if s == "posts"));
        assert!(matches!(&segs[3], SegmentPattern::Param(s) if s == "postId"));
    }

    #[test]
    fn test_match_single_route_non_exact_prefix() {
        let m = match_single_route("/", "/anything", false).unwrap();
        assert!(m.params.is_empty());
    }

    // ── Consumed segments tests ──

    #[test]
    fn test_consumed_segments_exact() {
        let m = match_single_route("/contacts/:id", "/contacts/42", true).unwrap();
        assert_eq!(m.consumed_segments, 2);
    }

    #[test]
    fn test_consumed_segments_prefix() {
        let m = match_single_route("/sections/:id", "/sections/1/topics/react", false).unwrap();
        assert_eq!(m.consumed_segments, 2);
    }

    #[test]
    fn test_consumed_segments_root() {
        let m = match_single_route("/", "/", true).unwrap();
        assert_eq!(m.consumed_segments, 0);
    }

    #[test]
    fn test_consumed_segments_splat() {
        let m = match_single_route("/files/*path", "/files/docs/readme.md", false).unwrap();
        assert_eq!(m.consumed_segments, 3);
    }

    // ── Relative path resolution tests ──

    #[test]
    fn test_is_relative_path() {
        assert!(is_relative_path("./sections/:id"));
        assert!(is_relative_path("./"));
        assert!(is_relative_path("sections/:id")); // bare relative
        assert!(is_relative_path("topics/:topicId")); // bare relative
        assert!(!is_relative_path("/sections/:id")); // absolute
        assert!(!is_relative_path("")); // empty
    }

    #[test]
    fn test_resolve_route_path_dotslash_relative() {
        assert_eq!(
            resolve_route_path("./topics/:id", "/sections/1"),
            "/sections/1/topics/:id"
        );
    }

    #[test]
    fn test_resolve_route_path_bare_relative() {
        assert_eq!(
            resolve_route_path("topics/:id", "/sections/1"),
            "/sections/1/topics/:id"
        );
    }

    #[test]
    fn test_resolve_route_path_relative_at_root() {
        assert_eq!(resolve_route_path("./sections/:id", "/"), "/sections/:id");
        assert_eq!(resolve_route_path("sections/:id", "/"), "/sections/:id");
    }

    #[test]
    fn test_resolve_route_path_absolute_unchanged() {
        assert_eq!(
            resolve_route_path("/sections/:id", "/some/base"),
            "/sections/:id"
        );
    }

    #[test]
    fn test_resolve_route_path_deep_nesting() {
        assert_eq!(
            resolve_route_path("./lessons/:lessonId", "/sections/1/topics/react"),
            "/sections/1/topics/react/lessons/:lessonId"
        );
        assert_eq!(
            resolve_route_path("lessons/:lessonId", "/sections/1/topics/react"),
            "/sections/1/topics/react/lessons/:lessonId"
        );
    }

    #[test]
    fn test_resolve_route_path_empty_resolves_to_base() {
        assert_eq!(resolve_route_path("", "/search"), "/search");
        assert_eq!(resolve_route_path("", "/"), "/");
        assert_eq!(resolve_route_path("", "/a/b/c"), "/a/b/c");
    }

    #[test]
    fn test_empty_child_route_matches_parent_path_exactly() {
        // Simulates <route path="search"> <route path="" exact /> when visiting /search
        let resolved = resolve_route_path("", "/search");
        let m = match_single_route(&resolved, "/search", true);
        assert!(m.is_some(), "empty child route must match parent path");
        let m = m.unwrap();
        assert_eq!(m.consumed_segments, 1);
    }

    // ── Compute route base tests ──

    #[test]
    fn test_compute_route_base_two_segments() {
        assert_eq!(
            compute_route_base("/sections/1/topics/react", 2),
            "/sections/1"
        );
    }

    #[test]
    fn test_compute_route_base_zero() {
        assert_eq!(compute_route_base("/sections/1", 0), "/");
    }

    #[test]
    fn test_compute_route_base_all_segments() {
        assert_eq!(
            compute_route_base("/sections/1/topics/react", 4),
            "/sections/1/topics/react"
        );
    }

    #[test]
    fn test_compute_route_base_exceeds_segments() {
        assert_eq!(compute_route_base("/sections/1", 10), "/sections/1");
    }

    #[test]
    fn test_param_sanitizes_traversal() {
        let m = match_single_route("/files/:name", "/files/..something", true).unwrap();
        assert!(!m.params["name"].contains(".."));
    }

    #[test]
    fn test_param_strips_null_bytes() {
        let m = match_single_route("/users/:id", "/users/test\0injected", true).unwrap();
        assert!(!m.params["id"].contains('\0'));
    }

    #[test]
    fn test_splat_sanitizes_traversal() {
        let m =
            match_single_route("/files/*path", "/files/docs/../../../etc/passwd", false).unwrap();
        assert!(!m.params["path"].contains(".."));
    }
}
