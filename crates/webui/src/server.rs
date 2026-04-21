// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! High-level server helper for custom Rust servers using `webui-router`.
//!
//! This module provides [`serve_request`] which encapsulates:
//! - Route parameter extraction from the URL
//! - HTML SSR rendering via [`WebUIHandler`]
//! - JSON partial responses for client-side navigation
//! - Template inventory management
//!
//! # Example
//!
//! ```rust,ignore
//! use webui::server::{serve_request, ServeRequest, ServeResponse};
//!
//! let request = ServeRequest {
//!     path: "/email/thread-5",
//!     accept_json: false,
//!     inventory_hex: "",
//! };
//!
//! match serve_request(&protocol, &handler, &state, "index.html", &request) {
//!     Ok(ServeResponse::Html(html)) => { /* serve HTML */ }
//!     Ok(ServeResponse::Json(json)) => { /* serve JSON */ }
//!     Err(e) => { /* handle error */ }
//! }
//! ```

use crate::{ResponseWriter, WebUIHandler};
use webui_handler::route_handler;
use webui_handler::RenderOptions;
use webui_protocol::WebUIProtocol;

/// A server request to be handled by [`serve_request`].
pub struct ServeRequest<'a> {
    /// The URL path (e.g., `"/email/thread-5"`, `"/folder/sent"`).
    pub path: &'a str,
    /// Whether the client accepts JSON (for partial navigation).
    /// Check `Accept: application/json` in request headers.
    pub accept_json: bool,
    /// The client's current template inventory (hex bitmask).
    /// Read from `X-WebUI-Inventory` request header. Empty string if not present.
    pub inventory_hex: &'a str,
}

/// The response from [`serve_request`].
pub enum ServeResponse {
    /// Full HTML page for initial load or browser refresh.
    Html(String),
    /// JSON partial for client-side navigation via `webui-router`.
    Json(serde_json::Value),
}

/// Handle a server request with automatic route handling.
///
/// For HTML requests: renders the full page with route-matched SSR,
/// injects `__webui_state` and `__webui_templates`.
///
/// For JSON requests: returns a partial response with route-scoped state,
/// needed templates, and inventory for the `webui-router` client.
///
/// # Arguments
/// - `protocol` — The compiled WebUI protocol from [`build`](crate::build)
/// - `handler` — The WebUI handler (with plugin configured)
/// - `state` — The state JSON to render. For HTML requests, this should be
///   the full app state. For JSON requests, the caller should provide
///   route-scoped state (only what the target page component needs).
/// - `request` — The incoming request details
///
/// # Route Parameters
/// Route parameters (`:param` in route paths) are automatically extracted
/// and injected into the state object.
pub fn serve_request(
    protocol: &WebUIProtocol,
    handler: &WebUIHandler,
    state: serde_json::Value,
    entry: &str,
    request: &ServeRequest<'_>,
) -> Result<ServeResponse, String> {
    // Extract route params and inject into state
    let params = route_handler::collect_nested_route_params(protocol, entry, request.path);
    let mut data = state;
    if let Some(map) = data.as_object_mut() {
        for (k, v) in &params {
            map.insert(k.clone(), serde_json::Value::String(v.clone()));
        }
    }

    if request.accept_json {
        // JSON partial response for client-side navigation.
        let mut partial =
            route_handler::render_partial(protocol, entry, request.path, request.inventory_hex);
        if let Some(obj) = partial.as_object_mut() {
            obj.insert("state".into(), data);
        }
        Ok(ServeResponse::Json(partial))
    } else {
        // Full HTML SSR
        let mut writer = MemWriter::with_capacity(131_072);
        let opts = RenderOptions::new(entry, request.path);
        handler
            .handle(protocol, &data, &opts, &mut writer)
            .map_err(|e| format!("render failed: {e}"))?;

        let state_json =
            serde_json::to_string(&data).map_err(|e| format!("serialize failed: {e}"))?;
        let safe_json = state_json.replace("</", "<\\/");
        let script = format!("<script>window.__webui_state={safe_json}</script>");
        let html = writer.buf.replace("</body>", &format!("{script}</body>"));

        Ok(ServeResponse::Html(html))
    }
}

struct MemWriter {
    buf: String,
}

impl MemWriter {
    fn with_capacity(cap: usize) -> Self {
        Self {
            buf: String::with_capacity(cap),
        }
    }
}

impl ResponseWriter for MemWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.buf.push_str(content);
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    use webui_protocol::{FragmentList, WebUIFragment};

    #[test]
    fn serve_request_json_partial_preserves_template_styles() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-page")],
            },
        );
        fragments.insert(
            "my-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>page</p>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        let component = protocol
            .components
            .entry("my-page".to_string())
            .or_default();
        component.template =
            "(function(){window.__webui_templates['my-page']={h:'<p>page</p>'};})();".to_string();
        component.css = ".page{color:red}".to_string();

        let handler = WebUIHandler::new();
        let request = ServeRequest {
            path: "/",
            accept_json: true,
            inventory_hex: "",
        };

        let response = serve_request(&protocol, &handler, json!({}), "index.html", &request)
            .expect("partial response should succeed");

        let json = match response {
            ServeResponse::Json(value) => value,
            ServeResponse::Html(_) => panic!("expected JSON partial response"),
        };

        // templateStyles must be present and non-empty for module-mode components
        assert_eq!(
            json["templateStyles"].as_array().map(Vec::len),
            Some(1),
            "serve_request should include module template styles"
        );
        // templates must not contain any style tags
        assert!(
            !json["templates"][0]
                .as_str()
                .unwrap_or_default()
                .contains("<style"),
            "template scripts should not contain module style tags"
        );
    }

    #[test]
    fn serve_request_link_strategy_returns_empty_template_styles() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-page")],
            },
        );
        fragments.insert(
            "my-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>page</p>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        let comp = protocol
            .components
            .entry("my-page".to_string())
            .or_default();
        comp.template = "(function(){})();".to_string();
        comp.css_href = "/my-page.css".to_string();
        // No css content — Link strategy

        let handler = WebUIHandler::new();
        let request = ServeRequest {
            path: "/",
            accept_json: true,
            inventory_hex: "",
        };

        let response = serve_request(&protocol, &handler, json!({}), "index.html", &request)
            .expect("partial response should succeed");

        let json = match response {
            ServeResponse::Json(value) => value,
            ServeResponse::Html(_) => panic!("expected JSON partial response"),
        };

        assert!(
            json["templateStyles"]
                .as_array()
                .is_some_and(|a| a.is_empty()),
            "Link strategy should return empty templateStyles"
        );
        assert!(
            json["templates"].as_array().is_some_and(|a| a.len() == 1),
            "Link strategy should still return templates"
        );
    }

    #[test]
    fn serve_request_style_strategy_returns_empty_template_styles() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-page")],
            },
        );
        fragments.insert(
            "my-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>page</p>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        let comp = protocol
            .components
            .entry("my-page".to_string())
            .or_default();
        // Style strategy: CSS is inlined in the template IIFE
        comp.template = "(function(){var w=window.__webui_templates;w['my-page']={h:'<style>.p{color:red}</style><p/>'};})();".to_string();

        let handler = WebUIHandler::new();
        let request = ServeRequest {
            path: "/",
            accept_json: true,
            inventory_hex: "",
        };

        let response = serve_request(&protocol, &handler, json!({}), "index.html", &request)
            .expect("partial response should succeed");

        let json = match response {
            ServeResponse::Json(value) => value,
            ServeResponse::Html(_) => panic!("expected JSON partial response"),
        };

        assert!(
            json["templateStyles"]
                .as_array()
                .is_some_and(|a| a.is_empty()),
            "Style strategy should return empty templateStyles"
        );
        assert!(
            json["templates"].as_array().is_some_and(|a| a.len() == 1),
            "Style strategy should still return templates"
        );
    }
}
