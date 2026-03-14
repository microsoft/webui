//! Route element parsing for `<route>` directives.
//!
//! Parses `<route path="..." component="..." ...>` elements into
//! `WebUIFragmentRoute` protocol fragments and builds a top-level
//! route registry (`HashMap<String, RouteRecord>`).

use std::collections::{HashMap, HashSet};

use webui_protocol::{RouteRecord, WebUiFragmentRoute};

use crate::error::{ParserError, Result};

/// Maximum number of path parameters allowed per route.
const MAX_PARAMS_PER_ROUTE: usize = 20;

/// Parsed attributes from a single `<route>` element.
#[derive(Debug, Default)]
pub(crate) struct RouteAttributes {
    pub path: String,
    pub component: String,
    pub name: String,
    pub exact: bool,
}

/// Iteratively extract `:param` and `*splat` tokens from a path template.
///
/// Returns param names without the `:` or `*` prefix.
/// Validates the path template for correctness.
pub(crate) fn extract_params(path: &str) -> Result<Vec<String>> {
    let mut params = Vec::new();
    let mut seen = HashSet::new();

    for segment in path.split('/') {
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
    // path is required
    if attrs.path.is_empty() {
        return Err(ParserError::Directive(
            "Route must have a non-empty 'path' attribute".into(),
        ));
    }

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
) -> WebUiFragmentRoute {
    WebUiFragmentRoute {
        path: attrs.path.clone(),
        fragment_id,
        exact: attrs.exact,
        name: attrs.name.clone(),
    }
}

/// Build a `RouteRecord` from a route fragment for the top-level registry.
pub(crate) fn build_route_record(route: &WebUiFragmentRoute) -> RouteRecord {
    RouteRecord {
        name: route.name.clone(),
        path: route.path.clone(),
        fragment_id: route.fragment_id.clone(),
        exact: route.exact,
    }
}

/// Route name uniqueness tracker.
pub(crate) struct RouteNameRegistry {
    names: HashSet<String>,
}

impl RouteNameRegistry {
    pub fn new() -> Self {
        Self {
            names: HashSet::new(),
        }
    }

    /// Register a route name, returning an error if it's a duplicate.
    pub fn register(&mut self, name: &str) -> Result<()> {
        if name.is_empty() {
            return Ok(());
        }
        if !self.names.insert(name.to_string()) {
            return Err(ParserError::Directive(format!(
                "Duplicate route name: '{name}'"
            )));
        }
        Ok(())
    }
}

/// Collect all route records into a map keyed by route name.
/// Routes without names are keyed by their fragment ID, with a counter
/// suffix appended when multiple unnamed routes share the same fragment ID.
pub(crate) fn collect_route_registry(
    routes: &[WebUiFragmentRoute],
) -> HashMap<String, RouteRecord> {
    let mut registry = HashMap::with_capacity(routes.len());
    let mut unnamed_counts: HashMap<String, usize> = HashMap::new();
    for route in routes {
        let key = if route.name.is_empty() {
            let count = unnamed_counts.entry(route.fragment_id.clone()).or_insert(0);
            *count += 1;
            if *count == 1 {
                route.fragment_id.clone()
            } else {
                format!("{}_{}", route.fragment_id, count)
            }
        } else {
            route.name.clone()
        };
        registry.insert(key, build_route_record(route));
    }
    registry
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

    #[test]
    fn test_validate_missing_path() {
        let attrs = RouteAttributes {
            component: "my-comp".to_string(),
            ..Default::default()
        };
        let err = validate_attributes(&attrs);
        assert!(err.is_err());
        assert!(
            err.unwrap_err().to_string().contains("path"),
            "Error should mention missing path"
        );
    }

    #[test]
    fn test_validate_missing_both() {
        let attrs = RouteAttributes::default();
        let err = validate_attributes(&attrs);
        assert!(err.is_err());
        // path is checked first
        assert!(
            err.unwrap_err().to_string().contains("path"),
            "Error should mention missing path first"
        );
    }

    #[test]
    fn test_route_name_registry_unique() {
        let mut reg = RouteNameRegistry::new();
        assert!(reg.register("home").is_ok());
        assert!(reg.register("profile").is_ok());
    }

    #[test]
    fn test_route_name_registry_duplicate() {
        let mut reg = RouteNameRegistry::new();
        assert!(reg.register("home").is_ok());
        assert!(reg.register("home").is_err());
    }

    #[test]
    fn test_route_name_registry_empty_ok() {
        let mut reg = RouteNameRegistry::new();
        assert!(reg.register("").is_ok());
        assert!(reg.register("").is_ok());
    }

    #[test]
    fn test_build_route_record() {
        let route = WebUiFragmentRoute {
            path: "/users/:id".to_string(),
            fragment_id: "users-page".to_string(),
            name: "user-detail".to_string(),
            exact: true,
        };
        let record = build_route_record(&route);
        assert_eq!(record.name, "user-detail");
        assert_eq!(record.path, "/users/:id");
        assert!(record.exact);
    }

    #[test]
    fn test_collect_route_registry() {
        let routes = vec![
            WebUiFragmentRoute {
                name: "home".to_string(),
                path: "/".to_string(),
                fragment_id: "home-page".to_string(),
                ..Default::default()
            },
            WebUiFragmentRoute {
                name: "".to_string(),
                path: "/unnamed".to_string(),
                fragment_id: "unnamed-page".to_string(),
                ..Default::default()
            },
        ];
        let registry = collect_route_registry(&routes);
        assert_eq!(registry.len(), 2);
        assert!(registry.contains_key("home"));
        assert!(registry.contains_key("unnamed-page"));
    }

    #[test]
    fn test_collect_route_registry_unnamed_collision() {
        let routes = vec![
            WebUiFragmentRoute {
                name: "".to_string(),
                path: "/a".to_string(),
                fragment_id: "shared-comp".to_string(),
                ..Default::default()
            },
            WebUiFragmentRoute {
                name: "".to_string(),
                path: "/b".to_string(),
                fragment_id: "shared-comp".to_string(),
                ..Default::default()
            },
        ];
        let registry = collect_route_registry(&routes);
        assert_eq!(
            registry.len(),
            2,
            "Both unnamed routes with the same fragment_id must be preserved"
        );
        assert!(registry.contains_key("shared-comp"));
        assert!(registry.contains_key("shared-comp_2"));
    }
}
