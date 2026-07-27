// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Iterative path template matcher for server-side route resolution.
//!
//! Matches request paths against route path templates without regex.
//! Supports `:param`, `:param?` (optional), and `*splat` segments.

use std::borrow::Cow;
use std::collections::HashMap;
use webui_protocol::{web_ui_fragment::Fragment, WebUIProtocol};

/// Result of matching a request path against a route path template.
#[derive(Debug, Clone)]
pub struct RouteMatch {
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

/// Immutable index of pre-compiled authored route patterns.
///
/// Absolute paths are matched from the request root. Relative paths reuse the
/// same compiled suffix and begin matching after the already-consumed parent
/// route segments, so request values never become cache keys.
#[derive(Debug)]
pub(crate) struct CompiledRouteIndex {
    patterns: HashMap<String, Vec<SegmentPattern>>,
}

impl CompiledRouteIndex {
    /// Compile every authored route path in the protocol.
    #[must_use]
    pub(crate) fn new(protocol: &WebUIProtocol) -> Self {
        let mut patterns = HashMap::new();
        let mut pending = Vec::new();

        for fragment_list in protocol.fragments.values() {
            for fragment in &fragment_list.fragments {
                if let Some(Fragment::Route(route)) = fragment.fragment.as_ref() {
                    pending.push(route);
                }
            }
        }

        while let Some(route) = pending.pop() {
            patterns
                .entry(route.path.clone())
                .or_insert_with(|| parse_template(&route.path));
            pending.extend(route.children.iter());
        }

        Self { patterns }
    }

    fn get(&self, template: &str) -> Option<&[SegmentPattern]> {
        self.patterns.get(template).map(Vec::as_slice)
    }
}

/// Match a route template against pre-split request path segments.
///
/// Relative templates are matched as precompiled suffixes after the parent
/// route base. This avoids constructing and compiling request-specific paths.
pub(crate) fn match_route_indexed_with_segments(
    index: &CompiledRouteIndex,
    template: &str,
    route_base: &str,
    request_segments: &[&str],
    exact: bool,
) -> Option<RouteMatch> {
    let patterns = index.get(template)?;
    let base_segments = if template.is_empty() || is_relative_path(template) {
        route_base
            .split('/')
            .filter(|segment| !segment.is_empty())
            .count()
    } else {
        0
    };
    let remaining = request_segments.get(base_segments..)?;
    let mut route_match = try_match(patterns, remaining, exact)?;
    route_match.specificity += base_segments;
    route_match.consumed_segments += base_segments;
    Some(route_match)
}

/// Split a request path into segments, filtering empty parts.
///
/// Intended to be called once before matching sibling routes.
pub fn split_request_path(path: &str) -> Vec<&str> {
    split_path(path)
}

/// Parse a path template string into segment patterns.
fn parse_template(template: &str) -> Vec<SegmentPattern> {
    let mut segments = Vec::new();
    let template = template.strip_prefix("./").unwrap_or(template);

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

/// Convert an ASCII hex digit to its numeric value.
pub(crate) fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Percent-decode a URL path segment (e.g. `%2F` → `/`, `%20` → ` `).
///
/// Returns `None` if the input contains malformed percent-encoding (a `%` not
/// followed by two hex digits) or if the decoded bytes are not valid UTF-8.
fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    if !bytes.contains(&b'%') {
        return Some(input.to_owned());
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hi = hex_val(bytes[i + 1])?;
            let lo = hex_val(bytes[i + 2])?;
            out.push(hi << 4 | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Validate and percent-decode a single route parameter segment.
///
/// Returns `None` (rejecting the match) if the segment:
/// - contains malformed percent-encoding,
/// - decodes to `..` (path traversal), or
/// - contains a NUL byte (`\0`).
fn validate_param(value: &str) -> Option<String> {
    let decoded = percent_decode(value)?;
    if decoded == ".." || decoded.bytes().any(|b| b == 0) {
        return None;
    }
    Some(decoded)
}

/// Try matching a request path against pre-parsed route template patterns.
///
/// Returns `Some(RouteMatch)` if the path matches, `None` otherwise.
fn try_match(
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
                params.insert(name.clone(), validate_param(request_segments[req_idx])?);
                req_idx += 1;
            }
            SegmentPattern::OptionalParam(name) => {
                if req_idx < request_segments.len() {
                    params.insert(name.clone(), validate_param(request_segments[req_idx])?);
                    req_idx += 1;
                }
                // Optional — ok to skip
            }
            SegmentPattern::Splat(name) => {
                // Splat consumes all remaining segments; validate each individually.
                let remaining = &request_segments[req_idx..];
                let mut decoded_parts = Vec::with_capacity(remaining.len());
                for seg in remaining {
                    decoded_parts.push(validate_param(seg)?);
                }
                params.insert(name.clone(), decoded_parts.join("/"));
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
    try_match(&patterns, &request_segments, exact)
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
    resolve_route_path_cow(path, route_base).into_owned()
}

/// Resolve a route path while borrowing absolute paths that need no changes.
pub(crate) fn resolve_route_path_cow<'a>(path: &'a str, route_base: &str) -> Cow<'a, str> {
    // An empty path means "match at the parent level" — resolve to base.
    if path.is_empty() {
        return Cow::Owned(route_base.to_string());
    }

    if !is_relative_path(path) {
        return Cow::Borrowed(path);
    }

    // Strip leading "./" if present
    let relative = path.strip_prefix("./").unwrap_or(path);
    if relative.is_empty() {
        return Cow::Owned(route_base.to_string());
    }

    let mut resolved = String::with_capacity(route_base.len() + relative.len() + 1);
    resolved.push_str(route_base);
    if !route_base.ends_with('/') {
        resolved.push('/');
    }
    resolved.push_str(relative);
    Cow::Owned(resolved)
}

/// Compute the new route base from a matched route's consumed request segments.
///
/// Given request path `/sections/1/topics/react` and consumed=2,
/// returns `/sections/1`.
pub fn compute_route_base(request_path: &str, consumed_segments: usize) -> String {
    if consumed_segments == 0 {
        return "/".to_string();
    }

    let mut base = String::with_capacity(request_path.len());
    let mut remaining = consumed_segments;
    for part in request_path
        .split('/')
        .filter(|segment| !segment.is_empty())
    {
        if remaining == 0 {
            break;
        }
        base.push('/');
        base.push_str(part);
        remaining -= 1;
    }
    if base.is_empty() {
        base.push('/');
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use webui_protocol::{FragmentList, WebUIFragment, WebUiFragmentRoute};

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
    fn test_param_rejects_traversal() {
        // Exact `..` segment is path traversal — must reject.
        assert!(match_single_route("/files/:name", "/files/..", true).is_none());
    }

    #[test]
    fn test_param_allows_double_dot_prefix() {
        // `..something` is NOT traversal (not a standalone `..` segment).
        let m = match_single_route("/files/:name", "/files/..something", true).unwrap();
        assert_eq!(m.params["name"], "..something");
    }

    #[test]
    fn test_param_rejects_null_bytes() {
        assert!(match_single_route("/users/:id", "/users/test\0injected", true).is_none());
    }

    #[test]
    fn test_splat_rejects_traversal() {
        assert!(
            match_single_route("/files/*path", "/files/docs/../../../etc/passwd", false).is_none()
        );
    }

    #[test]
    fn test_param_rejects_encoded_traversal() {
        // %2e%2e decodes to `..`
        assert!(match_single_route("/files/:name", "/files/%2e%2e", true).is_none());
    }

    #[test]
    fn test_param_rejects_mixed_encoded_traversal() {
        // .%2e decodes to `..`
        assert!(match_single_route("/files/:name", "/files/.%2e", true).is_none());
        // %2e. decodes to `..`
        assert!(match_single_route("/files/:name", "/files/%2e.", true).is_none());
    }

    #[test]
    fn test_param_rejects_encoded_null() {
        assert!(match_single_route("/users/:id", "/users/test%00injected", true).is_none());
    }

    #[test]
    fn test_param_rejects_malformed_percent() {
        assert!(match_single_route("/users/:id", "/users/100%", true).is_none());
        assert!(match_single_route("/users/:id", "/users/100%ZZ", true).is_none());
    }

    #[test]
    fn test_param_decodes_valid_percent_encoding() {
        let m = match_single_route("/files/:name", "/files/hello%20world", true).unwrap();
        assert_eq!(m.params["name"], "hello world");
    }

    #[test]
    fn test_splat_decodes_segments() {
        let m = match_single_route("/files/*path", "/files/my%20docs/read%20me.md", false).unwrap();
        assert_eq!(m.params["path"], "my docs/read me.md");
    }

    #[test]
    fn test_splat_rejects_encoded_traversal() {
        assert!(
            match_single_route("/files/*path", "/files/docs/%2e%2e/etc/passwd", false).is_none()
        );
    }

    // ── Parity golden tests ──
    // These lock down route-matching semantics that must stay consistent
    // between the Rust server and the TypeScript client.

    #[test]
    fn parity_root_exact() {
        let m = match_single_route("/", "/", true).unwrap();
        assert!(m.params.is_empty());
        assert_eq!(m.consumed_segments, 0);
        assert_eq!(m.specificity, 0);
    }

    #[test]
    fn parity_root_prefix_matches_anything() {
        let m = match_single_route("/", "/any/path/here", false).unwrap();
        assert_eq!(m.consumed_segments, 0);
    }

    #[test]
    fn parity_root_exact_rejects_non_root() {
        assert!(match_single_route("/", "/something", true).is_none());
    }

    #[test]
    fn parity_optional_param_present() {
        let m = match_single_route("/search/:query?", "/search/hello", true).unwrap();
        assert_eq!(m.params["query"], "hello");
        assert_eq!(m.consumed_segments, 2);
    }

    #[test]
    fn parity_optional_param_absent() {
        let m = match_single_route("/search/:query?", "/search", true).unwrap();
        assert!(!m.params.contains_key("query"));
        assert_eq!(m.consumed_segments, 1);
    }

    #[test]
    fn parity_splat_captures_all_remaining() {
        let m = match_single_route("/files/*path", "/files/a/b/c/d.txt", false).unwrap();
        assert_eq!(m.params["path"], "a/b/c/d.txt");
        assert_eq!(m.consumed_segments, 5);
    }

    #[test]
    fn parity_splat_captures_empty() {
        let m = match_single_route("/files/*path", "/files", false).unwrap();
        assert_eq!(m.params["path"], "");
        assert_eq!(m.consumed_segments, 1);
    }

    #[test]
    fn parity_unnamed_splat_defaults_to_rest() {
        let m = match_single_route("/files/*", "/files/a/b", false).unwrap();
        assert_eq!(m.params["rest"], "a/b");
    }

    #[test]
    fn parity_percent_encoded_space() {
        let m = match_single_route("/items/:name", "/items/hello%20world", true).unwrap();
        assert_eq!(m.params["name"], "hello world");
    }

    #[test]
    fn parity_percent_encoded_slash() {
        let m = match_single_route("/items/:name", "/items/a%2Fb", true).unwrap();
        assert_eq!(m.params["name"], "a/b");
    }

    #[test]
    fn parity_unicode_path() {
        let m = match_single_route("/pages/:title", "/pages/caf%C3%A9", true).unwrap();
        assert_eq!(m.params["title"], "café");
    }

    #[test]
    fn parity_specificity_literal_beats_param() {
        // "/contacts/add" (specificity 2) must beat "/contacts/:id" (specificity 1)
        let add = match_single_route("/contacts/add", "/contacts/add", true).unwrap();
        let param = match_single_route("/contacts/:id", "/contacts/add", true).unwrap();
        assert!(add.specificity > param.specificity);
    }

    #[test]
    fn parity_equal_specificity_first_wins() {
        // Two patterns with same specificity — first declared wins.
        // This test validates the contract at the caller level (find_best_route_match).
        let m1 = match_single_route("/items/:a", "/items/x", true).unwrap();
        let m2 = match_single_route("/items/:b", "/items/x", true).unwrap();
        assert_eq!(m1.specificity, m2.specificity, "equal specificity expected");
    }

    #[test]
    fn parity_relative_path_resolution() {
        assert_eq!(
            resolve_route_path("./topics/:id", "/sections/1"),
            "/sections/1/topics/:id"
        );
        assert_eq!(
            resolve_route_path("topics/:id", "/sections/1"),
            "/sections/1/topics/:id"
        );
        assert_eq!(resolve_route_path("/absolute", "/any/base"), "/absolute");
        assert_eq!(resolve_route_path("", "/base"), "/base");
        assert_eq!(resolve_route_path("./", "/base"), "/base");
    }

    #[test]
    fn parity_compute_route_base_various() {
        assert_eq!(compute_route_base("/", 0), "/");
        assert_eq!(compute_route_base("/a/b/c", 1), "/a");
        assert_eq!(compute_route_base("/a/b/c", 2), "/a/b");
        assert_eq!(compute_route_base("/a/b/c", 3), "/a/b/c");
        assert_eq!(compute_route_base("/a/b/c", 99), "/a/b/c");
    }

    #[test]
    fn parity_nested_route_matching() {
        // Simulate: parent route "/sections/:id" matches "/sections/1/topics/react"
        let parent =
            match_single_route("/sections/:id", "/sections/1/topics/react", false).unwrap();
        assert_eq!(parent.params["id"], "1");
        assert_eq!(parent.consumed_segments, 2);

        // Child route is relative: "topics/:topicId" resolved against base "/sections/1"
        let base = compute_route_base("/sections/1/topics/react", parent.consumed_segments);
        assert_eq!(base, "/sections/1");

        let child_path = resolve_route_path("./topics/:topicId", &base);
        assert_eq!(child_path, "/sections/1/topics/:topicId");

        let child = match_single_route(&child_path, "/sections/1/topics/react", true).unwrap();
        assert_eq!(child.params["topicId"], "react");
        assert_eq!(child.consumed_segments, 4);
    }

    #[test]
    fn compiled_index_matches_relative_suffix_without_request_key() {
        let child = WebUiFragmentRoute {
            path: "topics/:topicId".to_string(),
            fragment_id: "topic-page".to_string(),
            exact: true,
            ..Default::default()
        };
        let parent = WebUiFragmentRoute {
            path: "/sections/:sectionId".to_string(),
            fragment_id: "section-page".to_string(),
            children: vec![child],
            ..Default::default()
        };
        let protocol = WebUIProtocol::new(HashMap::from([(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(parent)],
            },
        )]));
        let index = CompiledRouteIndex::new(&protocol);
        let request_segments = split_request_path("/sections/1/topics/react");

        let route_match = match_route_indexed_with_segments(
            &index,
            "topics/:topicId",
            "/sections/1",
            &request_segments,
            true,
        )
        .expect("relative child route should match");

        assert_eq!(route_match.params["topicId"], "react");
        assert_eq!(route_match.consumed_segments, 4);
        assert_eq!(route_match.specificity, 3);
        assert_eq!(index.patterns.len(), 2);
    }

    #[test]
    fn parity_non_exact_prefix_with_extra_segments() {
        let m = match_single_route("/app", "/app/dashboard/settings", false).unwrap();
        assert_eq!(m.consumed_segments, 1);
        assert_eq!(m.specificity, 1);
    }

    #[test]
    fn parity_exact_rejects_extra_segments() {
        assert!(match_single_route("/app", "/app/dashboard", true).is_none());
    }

    #[test]
    fn parity_multiple_optional_params() {
        let m = match_single_route("/search/:q?/:page?", "/search", true).unwrap();
        assert!(!m.params.contains_key("q"));
        assert!(!m.params.contains_key("page"));

        let m = match_single_route("/search/:q?/:page?", "/search/rust", true).unwrap();
        assert_eq!(m.params["q"], "rust");
        assert!(!m.params.contains_key("page"));

        let m = match_single_route("/search/:q?/:page?", "/search/rust/2", true).unwrap();
        assert_eq!(m.params["q"], "rust");
        assert_eq!(m.params["page"], "2");
    }
}
