// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Route element parsing for `<route>` directives.
//!
//! Parses `<route path="..." component="..." ...>` elements into
//! `WebUIFragmentRoute` protocol fragments.

use std::collections::HashSet;

use webui_protocol::WebUiFragmentRoute;

use crate::error::{ParserError, Result};

/// Maximum number of path parameters allowed per route.
const MAX_PARAMS_PER_ROUTE: usize = 20;

/// Parsed attributes from a single `<route>` element.
#[derive(Debug, Default)]
pub(crate) struct RouteAttributes {
    pub path: String,
    pub component: String,
    pub exact: bool,
    /// Comma-separated allowlist of query parameters forwarded as attributes.
    pub query: String,
    /// When true, the router keeps the component alive across navigations.
    pub keep_alive: bool,
    /// Cache tag templates (e.g. `["thread:{threadId}", "inbox"]`).
    /// Placeholders like `{param}` are resolved at render time.
    pub cache_tags: Vec<String>,
    /// Invalidation tag templates (e.g. `["inbox", "sent"]`).
    /// After a mutation action, these tags are auto-invalidated.
    /// Supports `{param}` placeholders resolved at render time.
    pub invalidates: Vec<String>,
    /// Component tag name for pending/loading UI.
    pub pending_component: String,
    /// Component tag name for error boundary UI.
    pub error_component: String,
}

/// Iteratively extract `:param` and `*splat` tokens from a path template.
///
/// Returns param names without the `:` or `*` prefix.
/// Validates the path template for correctness.
/// Handles relative paths (starting with `./`) by stripping the prefix
/// before parameter extraction.
pub(crate) fn extract_params(path: &str) -> Result<Vec<String>> {
    // Strip relative prefix — params are the same regardless
    let normalized = path.strip_prefix("./").unwrap_or(path);

    let mut params = Vec::new();
    let mut seen = HashSet::new();

    for segment in normalized.split('/') {
        if segment.is_empty() {
            continue;
        }

        // :param or :param? (optional)
        if let Some(rest) = segment.strip_prefix(':') {
            let name = rest.trim_end_matches('?');
            if name.is_empty() {
                return Err(ParserError::Directive(format!(
                    "Empty parameter name in path: {path}"
                )));
            }
            if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(ParserError::Directive(format!(
                    "Invalid parameter name '{name}' in path: {path}"
                )));
            }
            if !seen.insert(name.to_string()) {
                return Err(ParserError::Directive(format!(
                    "Duplicate parameter name '{name}' in path: {path}"
                )));
            }
            params.push(name.to_string());
        }
        // *splat
        else if let Some(rest) = segment.strip_prefix('*') {
            let name = if rest.is_empty() { "rest" } else { rest };
            if !seen.insert(name.to_string()) {
                return Err(ParserError::Directive(format!(
                    "Duplicate splat name '{name}' in path: {path}"
                )));
            }
            params.push(name.to_string());
        }
    }

    if params.len() > MAX_PARAMS_PER_ROUTE {
        return Err(ParserError::Directive(format!(
            "Too many parameters ({}) in path: {path} (max {MAX_PARAMS_PER_ROUTE})",
            params.len()
        )));
    }

    Ok(params)
}

/// Validate route attributes for consistency.
pub(crate) fn validate_attributes(attrs: &RouteAttributes) -> Result<()> {
    // component is required
    if attrs.component.is_empty() {
        return Err(ParserError::Directive(format!(
            "Route '{}' must have a 'component' attribute",
            attrs.path
        )));
    }

    Ok(())
}

/// Parse a comma-separated list of cache tags or invalidation tags.
///
/// Splits on `,`, trims whitespace, and discards empty entries.
pub(crate) fn parse_tag_list(raw: &str) -> Vec<String> {
    let mut tags = Vec::new();
    for part in raw.split(',') {
        let trimmed = part.trim();
        if !trimmed.is_empty() {
            tags.push(trimmed.to_string());
        }
    }
    tags
}

/// Extract `{param}` placeholder names from tag templates.
///
/// Returns the set of param names referenced in tags like `"thread:{threadId}"`.
/// Validates that placeholder syntax is well-formed.
pub(crate) fn extract_tag_placeholders(tags: &[String]) -> Result<HashSet<String>> {
    let mut placeholders = HashSet::new();

    for tag in tags {
        let bytes = tag.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        while i < len {
            if bytes[i] == b'{' {
                let start = i + 1;
                let end = tag[start..].find('}').map(|j| start + j);
                match end {
                    Some(end_idx) => {
                        let name = &tag[start..end_idx];
                        if name.is_empty() {
                            return Err(ParserError::Directive(format!(
                                "Empty placeholder '{{}}' in cache tag: {tag}"
                            )));
                        }
                        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                            return Err(ParserError::Directive(format!(
                                "Invalid placeholder name '{name}' in cache tag: {tag}"
                            )));
                        }
                        placeholders.insert(name.to_string());
                        i = end_idx + 1;
                    }
                    None => {
                        return Err(ParserError::Directive(format!(
                            "Unclosed placeholder '{{' in cache tag: {tag}"
                        )));
                    }
                }
            } else {
                i += 1;
            }
        }
    }

    Ok(placeholders)
}

/// Validate that `{param}` placeholders in tags reference actual route params.
///
/// `available_params` should include params from this route AND all ancestor routes.
pub(crate) fn validate_tag_placeholders(
    tags: &[String],
    available_params: &HashSet<String>,
    attr_name: &str,
    route_path: &str,
) -> Result<()> {
    let placeholders = extract_tag_placeholders(tags)?;
    for name in &placeholders {
        if !available_params.contains(name) {
            return Err(ParserError::Directive(format!(
                "Placeholder '{{{name}}}' in {attr_name} references unknown param \
                 on route '{route_path}'. Available params: {available}",
                available = if available_params.is_empty() {
                    "(none)".to_string()
                } else {
                    let mut sorted: Vec<&str> =
                        available_params.iter().map(|s| s.as_str()).collect();
                    sorted.sort_unstable();
                    sorted.join(", ")
                }
            )));
        }
    }
    Ok(())
}

/// Build a `WebUiFragmentRoute` from parsed attributes.
pub(crate) fn build_route_fragment(
    attrs: &RouteAttributes,
    fragment_id: String,
    children: Vec<WebUiFragmentRoute>,
) -> WebUiFragmentRoute {
    WebUiFragmentRoute {
        path: attrs.path.clone(),
        fragment_id,
        exact: attrs.exact,
        children,
        allowed_query: attrs.query.clone(),
        keep_alive: attrs.keep_alive,
        cache_tags: attrs.cache_tags.clone(),
        invalidates: attrs.invalidates.clone(),
        pending_component: attrs.pending_component.clone(),
        error_component: attrs.error_component.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_params_simple() {
        let params = extract_params("/profile/:id").unwrap();
        assert_eq!(params, vec!["id"]);
    }

    #[test]
    fn test_extract_params_multiple() {
        let params = extract_params("/profile/:id/view/:section").unwrap();
        assert_eq!(params, vec!["id", "section"]);
    }

    #[test]
    fn test_extract_params_optional() {
        let params = extract_params("/profile/:id?").unwrap();
        assert_eq!(params, vec!["id"]);
    }

    #[test]
    fn test_extract_params_splat() {
        let params = extract_params("/files/*rest").unwrap();
        assert_eq!(params, vec!["rest"]);
    }

    #[test]
    fn test_extract_params_splat_unnamed() {
        let params = extract_params("/files/*").unwrap();
        assert_eq!(params, vec!["rest"]);
    }

    #[test]
    fn test_extract_params_empty_path() {
        let params = extract_params("/").unwrap();
        assert!(params.is_empty());
    }

    #[test]
    fn test_extract_params_no_params() {
        let params = extract_params("/contacts/add").unwrap();
        assert!(params.is_empty());
    }

    #[test]
    fn test_extract_params_duplicate_error() {
        let result = extract_params("/profile/:id/other/:id");
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_params_invalid_name() {
        let result = extract_params("/profile/:id-with-dash");
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_params_empty_name() {
        let result = extract_params("/profile/:");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_attributes_valid() {
        let attrs = RouteAttributes {
            path: "/profile".to_string(),
            component: "profile-page".to_string(),
            ..Default::default()
        };
        assert!(validate_attributes(&attrs).is_ok());
    }

    #[test]
    fn test_validate_missing_component() {
        let attrs = RouteAttributes {
            path: "/page".to_string(),
            ..Default::default()
        };
        assert!(validate_attributes(&attrs).is_err());
    }

    // ── Relative path tests ──

    #[test]
    fn test_extract_params_relative_path() {
        let params = extract_params("./sections/:id").unwrap();
        assert_eq!(params, vec!["id"]);
    }

    #[test]
    fn test_extract_params_relative_multiple() {
        let params = extract_params("./topics/:topicId/lessons/:lessonId").unwrap();
        assert_eq!(params, vec!["topicId", "lessonId"]);
    }

    #[test]
    fn test_extract_params_relative_splat() {
        let params = extract_params("./*rest").unwrap();
        assert_eq!(params, vec!["rest"]);
    }

    // ── Cache tag parsing tests ──

    #[test]
    fn test_parse_tag_list_simple() {
        let tags = parse_tag_list("inbox,counts");
        assert_eq!(tags, vec!["inbox", "counts"]);
    }

    #[test]
    fn test_parse_tag_list_with_placeholders() {
        let tags = parse_tag_list("thread:{threadId},inbox");
        assert_eq!(tags, vec!["thread:{threadId}", "inbox"]);
    }

    #[test]
    fn test_parse_tag_list_whitespace() {
        let tags = parse_tag_list(" inbox , counts , drafts ");
        assert_eq!(tags, vec!["inbox", "counts", "drafts"]);
    }

    #[test]
    fn test_parse_tag_list_empty() {
        let tags = parse_tag_list("");
        assert!(tags.is_empty());
    }

    #[test]
    fn test_parse_tag_list_trailing_comma() {
        let tags = parse_tag_list("inbox,");
        assert_eq!(tags, vec!["inbox"]);
    }

    #[test]
    fn test_extract_tag_placeholders_simple() {
        let tags = vec!["thread:{threadId}".to_string(), "inbox".to_string()];
        let placeholders = extract_tag_placeholders(&tags).unwrap();
        assert_eq!(placeholders.len(), 1);
        assert!(placeholders.contains("threadId"));
    }

    #[test]
    fn test_extract_tag_placeholders_multiple() {
        let tags = vec!["folder:{folderId}".to_string(), "user:{userId}".to_string()];
        let placeholders = extract_tag_placeholders(&tags).unwrap();
        assert_eq!(placeholders.len(), 2);
        assert!(placeholders.contains("folderId"));
        assert!(placeholders.contains("userId"));
    }

    #[test]
    fn test_extract_tag_placeholders_none() {
        let tags = vec!["inbox".to_string(), "counts".to_string()];
        let placeholders = extract_tag_placeholders(&tags).unwrap();
        assert!(placeholders.is_empty());
    }

    #[test]
    fn test_extract_tag_placeholders_unclosed_brace() {
        let tags = vec!["thread:{threadId".to_string()];
        assert!(extract_tag_placeholders(&tags).is_err());
    }

    #[test]
    fn test_extract_tag_placeholders_empty_placeholder() {
        let tags = vec!["thread:{}".to_string()];
        assert!(extract_tag_placeholders(&tags).is_err());
    }

    #[test]
    fn test_extract_tag_placeholders_invalid_name() {
        let tags = vec!["thread:{thread-id}".to_string()];
        assert!(extract_tag_placeholders(&tags).is_err());
    }

    #[test]
    fn test_validate_tag_placeholders_valid() {
        let tags = vec!["thread:{threadId}".to_string(), "inbox".to_string()];
        let mut params = HashSet::new();
        params.insert("threadId".to_string());
        assert!(
            validate_tag_placeholders(&tags, &params, "cache-tags", "/email/:threadId").is_ok()
        );
    }

    #[test]
    fn test_validate_tag_placeholders_missing_param() {
        let tags = vec!["thread:{threadId}".to_string()];
        let params = HashSet::new();
        let result = validate_tag_placeholders(&tags, &params, "cache-tags", "/email");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("threadId"),
            "Error should mention the param: {err}"
        );
    }

    #[test]
    fn test_validate_tag_placeholders_with_ancestor_params() {
        let tags = vec![
            "thread:{threadId}".to_string(),
            "folder:{folderId}".to_string(),
        ];
        let mut params = HashSet::new();
        params.insert("threadId".to_string());
        params.insert("folderId".to_string());
        assert!(
            validate_tag_placeholders(&tags, &params, "cache-tags", "/email/:threadId").is_ok()
        );
    }
}
