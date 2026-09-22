// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::sync::Arc;

use percent_encoding::percent_decode_str;
use serde_json::Value;

use crate::{
    DesktopError, DesktopHttpMethod, DesktopProtocolRequest, DesktopProtocolResponse, Result,
};

type RouteStateHandler = dyn Fn(RouteContext<'_>) -> Result<Value> + Send + Sync;
type ApiHandler = dyn Fn(ApiContext<'_>) -> Result<DesktopProtocolResponse> + Send + Sync;

/// Registry of Rust route state providers.
#[derive(Default)]
pub struct RouteStateRegistry {
    routes: Vec<RouteStateEntry>,
}

/// Registry of Rust custom-protocol API handlers.
#[derive(Default)]
pub struct ApiRouteRegistry {
    routes: Vec<ApiRouteEntry>,
}

impl ApiRouteRegistry {
    /// Create an empty API route registry.
    #[must_use]
    pub fn new() -> Self {
        Self { routes: Vec::new() }
    }

    /// Register an API handler for a URL path pattern.
    ///
    /// # Errors
    ///
    /// Returns [`DesktopError`] if `pattern` is invalid.
    pub fn route<F>(&mut self, pattern: impl AsRef<str>, handler: F) -> Result<()>
    where
        F: Fn(ApiContext<'_>) -> Result<DesktopProtocolResponse> + Send + Sync + 'static,
    {
        self.routes.push(ApiRouteEntry {
            pattern: RoutePattern::parse(pattern.as_ref())?,
            handler: Arc::new(handler),
        });
        Ok(())
    }

    pub(crate) fn resolve(
        &self,
        request: &DesktopProtocolRequest<'_>,
    ) -> Result<Option<DesktopProtocolResponse>> {
        let path = route_path(request.path);
        for entry in &self.routes {
            let Some(params) = entry.pattern.matches(path) else {
                continue;
            };
            return (entry.handler)(ApiContext {
                method: &request.method,
                path,
                params: &params,
                body: request.body,
            })
            .map(Some);
        }
        Ok(None)
    }
}

struct ApiRouteEntry {
    pattern: RoutePattern,
    handler: Arc<ApiHandler>,
}

impl RouteStateRegistry {
    pub(crate) fn has_provider(&self, path: &str) -> bool {
        self.routes
            .iter()
            .any(|entry| entry.pattern.matches(path).is_some())
    }

    /// Create an empty route state registry.
    #[must_use]
    pub fn new() -> Self {
        Self { routes: Vec::new() }
    }

    /// Register a route state provider.
    ///
    /// # Errors
    ///
    /// Returns [`DesktopError`] if `pattern` is invalid.
    pub fn route<F>(&mut self, pattern: impl AsRef<str>, handler: F) -> Result<()>
    where
        F: Fn(RouteContext<'_>) -> Result<Value> + Send + Sync + 'static,
    {
        self.routes.push(RouteStateEntry {
            pattern: RoutePattern::parse(pattern.as_ref())?,
            handler: Arc::new(handler),
        });
        Ok(())
    }

    pub(crate) fn resolve(&self, path: &str, base_state: &Value) -> Result<Option<Value>> {
        for entry in &self.routes {
            let Some(params) = entry.pattern.matches(path) else {
                continue;
            };
            return (entry.handler)(RouteContext {
                path,
                params: &params,
                base_state,
            })
            .map(Some)
            .map_err(|err| DesktopError::RouteProvider {
                path: path.to_string(),
                message: err.chain_message(),
            });
        }
        Ok(None)
    }
}

struct RouteStateEntry {
    pattern: RoutePattern,
    handler: Arc<RouteStateHandler>,
}

struct RoutePattern {
    segments: Vec<RouteSegment>,
}

enum RouteSegment {
    Literal(String),
    Param(String),
}

/// Context passed to Rust route state providers.
pub struct RouteContext<'a> {
    /// Request path without query string.
    pub path: &'a str,
    params: &'a [(String, String)],
    /// File-backed base state loaded by the desktop runtime.
    pub base_state: &'a Value,
}

/// Context passed to Rust custom-protocol API handlers.
pub struct ApiContext<'a> {
    /// Request method.
    pub method: &'a DesktopHttpMethod,
    /// Request path without query string.
    pub path: &'a str,
    params: &'a [(String, String)],
    /// Request body bytes.
    pub body: &'a [u8],
}

impl ApiContext<'_> {
    /// Return a route parameter by name.
    #[must_use]
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find_map(|(key, value)| (key == name).then_some(value.as_str()))
    }
}

impl RouteContext<'_> {
    /// Return a route parameter by name.
    #[must_use]
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find_map(|(key, value)| (key == name).then_some(value.as_str()))
    }
}

impl RoutePattern {
    fn parse(pattern: &str) -> Result<Self> {
        if !pattern.starts_with('/') {
            return Err(DesktopError::InvalidRoutePattern {
                pattern: pattern.to_string(),
                help: "desktop route patterns must start with '/', e.g. /contacts/:id".to_string(),
            });
        }
        let trimmed = pattern.trim_matches('/');
        let mut segments = Vec::new();
        if !trimmed.is_empty() {
            for segment in trimmed.split('/') {
                if segment.is_empty() || segment == "." || segment == ".." {
                    return Err(DesktopError::InvalidRoutePattern {
                        pattern: pattern.to_string(),
                        help: "route pattern segments cannot be empty, '.', or '..'".to_string(),
                    });
                }
                if let Some(param) = segment.strip_prefix(':') {
                    if param.is_empty() {
                        return Err(DesktopError::InvalidRoutePattern {
                            pattern: pattern.to_string(),
                            help: "route parameter names cannot be empty".to_string(),
                        });
                    }
                    segments.push(RouteSegment::Param(param.to_string()));
                } else {
                    segments.push(RouteSegment::Literal(segment.to_string()));
                }
            }
        }
        Ok(Self { segments })
    }

    fn matches(&self, path: &str) -> Option<Vec<(String, String)>> {
        let trimmed = route_path(path).trim_matches('/');
        if trimmed.is_empty() {
            return self.segments.is_empty().then(Vec::new);
        }
        if trimmed.split('/').count() != self.segments.len() {
            return None;
        }
        let mut params = Vec::new();
        for (pattern, raw_segment) in self.segments.iter().zip(trimmed.split('/')) {
            let segment = percent_decode_str(raw_segment)
                .decode_utf8()
                .map(|value| value.into_owned())
                .unwrap_or_else(|_| raw_segment.to_string());
            match pattern {
                RouteSegment::Literal(expected) if expected == &segment => {}
                RouteSegment::Literal(_) => return None,
                RouteSegment::Param(name) => params.push((name.clone(), segment)),
            }
        }
        Some(params)
    }
}

pub(crate) fn route_path(request_path: &str) -> &str {
    request_path
        .split_once('?')
        .map_or(request_path, |(path, _)| path)
}
