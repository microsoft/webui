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
                params.insert(name.clone(), request_segments[req_idx].to_string());
                req_idx += 1;
            }
            SegmentPattern::OptionalParam(name) => {
                if req_idx < request_segments.len() {
                    params.insert(name.clone(), request_segments[req_idx].to_string());
                    req_idx += 1;
                }
                // Optional — ok to skip
            }
            SegmentPattern::Splat(name) => {
                // Splat consumes all remaining segments
                let remaining: Vec<&str> = request_segments[req_idx..].to_vec();
                params.insert(name.clone(), remaining.join("/"));
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
    })
}

/// Match a request path against a set of routes.
///
/// Returns the best match: exact matches preferred, then highest specificity
/// (most literal segments matched), then first defined.
pub fn match_route(
    routes: &HashMap<String, webui_protocol::RouteRecord>,
    request_path: &str,
) -> Option<RouteMatch> {
    let request_segments = split_path(request_path);
    let mut best_match: Option<RouteMatch> = None;

    for (key, route) in routes {
        let patterns = parse_template(&route.path);
        let matched = try_match(key, &patterns, &request_segments, route.exact);

        if let Some(m) = matched {
            let is_better = match &best_match {
                None => true,
                Some(current) => m.specificity > current.specificity,
            };

            if is_better {
                best_match = Some(m);
            }
        }
    }

    best_match
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

#[cfg(test)]
mod tests {
    use super::*;
    use webui_protocol::RouteRecord;

    fn make_route(name: &str, path: &str, exact: bool, _params: &[&str]) -> (String, RouteRecord) {
        (
            name.to_string(),
            RouteRecord {
                name: name.to_string(),
                path: path.to_string(),
                fragment_id: format!("{name}-page"),
                exact,
            },
        )
    }

    #[test]
    fn test_exact_root_match() {
        let mut routes = HashMap::new();
        routes.insert("home".into(), make_route("home", "/", true, &[]).1);
        let m = match_route(&routes, "/").unwrap();
        assert_eq!(m.route_key, "home");
        assert!(m.params.is_empty());
    }

    #[test]
    fn test_exact_static_path() {
        let mut routes = HashMap::new();
        routes.insert(
            "contacts".into(),
            make_route("contacts", "/contacts", true, &[]).1,
        );
        let m = match_route(&routes, "/contacts").unwrap();
        assert_eq!(m.route_key, "contacts");
    }

    #[test]
    fn test_exact_no_match_with_extra_segments() {
        let mut routes = HashMap::new();
        routes.insert(
            "contacts".into(),
            make_route("contacts", "/contacts", true, &[]).1,
        );
        assert!(match_route(&routes, "/contacts/123").is_none());
    }

    #[test]
    fn test_param_match() {
        let mut routes = HashMap::new();
        routes.insert(
            "detail".into(),
            make_route("detail", "/contacts/:id", true, &["id"]).1,
        );
        let m = match_route(&routes, "/contacts/42").unwrap();
        assert_eq!(m.route_key, "detail");
        assert_eq!(m.params["id"], "42");
    }

    #[test]
    fn test_multiple_params() {
        let mut routes = HashMap::new();
        routes.insert(
            "profile-view".into(),
            make_route(
                "profile-view",
                "/profile/:id/view/:section",
                true,
                &["id", "section"],
            )
            .1,
        );
        let m = match_route(&routes, "/profile/123/view/bio").unwrap();
        assert_eq!(m.params["id"], "123");
        assert_eq!(m.params["section"], "bio");
    }

    #[test]
    fn test_splat_match() {
        let mut routes = HashMap::new();
        routes.insert(
            "files".into(),
            make_route("files", "/files/*path", false, &["path"]).1,
        );
        let m = match_route(&routes, "/files/docs/readme.md").unwrap();
        assert_eq!(m.params["path"], "docs/readme.md");
    }

    #[test]
    fn test_optional_param_present() {
        let mut routes = HashMap::new();
        routes.insert(
            "search".into(),
            make_route("search", "/search/:query?", true, &["query"]).1,
        );
        let m = match_route(&routes, "/search/hello").unwrap();
        assert_eq!(m.params["query"], "hello");
    }

    #[test]
    fn test_optional_param_absent() {
        let mut routes = HashMap::new();
        routes.insert(
            "search".into(),
            make_route("search", "/search/:query?", true, &["query"]).1,
        );
        let m = match_route(&routes, "/search").unwrap();
        assert!(!m.params.contains_key("query"));
    }

    #[test]
    fn test_specificity_prefers_exact_literals() {
        let mut routes = HashMap::new();
        routes.insert(
            "add".into(),
            make_route("add", "/contacts/add", true, &[]).1,
        );
        routes.insert(
            "detail".into(),
            make_route("detail", "/contacts/:id", true, &["id"]).1,
        );
        let m = match_route(&routes, "/contacts/add").unwrap();
        assert_eq!(m.route_key, "add");
    }

    #[test]
    fn test_no_match() {
        let mut routes = HashMap::new();
        routes.insert("home".into(), make_route("home", "/", true, &[]).1);
        assert!(match_route(&routes, "/nonexistent").is_none());
    }

    #[test]
    fn test_non_exact_prefix_match() {
        let mut routes = HashMap::new();
        routes.insert("app".into(), make_route("app", "/", false, &[]).1);
        // Non-exact "/" should match any path as a prefix
        let m = match_route(&routes, "/anything").unwrap();
        assert_eq!(m.route_key, "app");
    }

    #[test]
    fn test_redirect_route_match() {
        let mut routes = HashMap::new();
        let mut redirect_route = make_route("old", "/old-path", true, &[]).1;
        redirect_route.fragment_id = String::new();
        routes.insert("old".into(), redirect_route);
        let m = match_route(&routes, "/old-path").unwrap();
        assert_eq!(m.route_key, "old");
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
    fn test_empty_path_matches_root() {
        let mut routes = HashMap::new();
        routes.insert("dashboard".into(), make_route("dashboard", "", true, &[]).1);
        let m = match_route(&routes, "/").unwrap();
        assert_eq!(m.route_key, "dashboard");
    }
}
