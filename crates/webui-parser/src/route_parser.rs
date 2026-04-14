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
}
