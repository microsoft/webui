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

use crate::{Protocol, ResponseWriter, WebUIHandler};
use webui_handler::route_handler;
use webui_handler::RenderOptions;

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
    Json(String),
}

/// Handle a server request with automatic route handling.
///
/// For HTML requests: renders the full page with route-matched SSR.
/// The handler emits a consolidated `window.__webui` script block
/// containing state, chain, inventory, and template metadata.
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
    protocol: &Protocol,
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
        let state_json =
            serde_json::to_string(&data).map_err(|e| format!("state serialization failed: {e}"))?;
        let partial = protocol
            .render_partial(&state_json, entry, request.path, request.inventory_hex)
            .map_err(|e| format!("render_partial failed: {e}"))?;
        Ok(ServeResponse::Json(partial))
    } else {
        // Full HTML SSR — handler emits the consolidated window.__webui
        // script block (state, chain, inventory, templates) automatically.
        let mut writer = MemWriter::with_capacity(131_072);
        let opts = RenderOptions::new(entry, request.path);
        handler
            .render(protocol, &data, &opts, &mut writer)
            .map_err(|e| format!("render failed: {e}"))?;

        Ok(ServeResponse::Html(writer.buf))
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
    use webui_protocol::{
        ComponentStyleClosure, CssStrategy, FragmentList, WebUIFragment, WebUIProtocol,
    };

    fn add_page_style_closures(protocol: &mut WebUIProtocol) {
        let closure = ComponentStyleClosure {
            component_tags: vec!["my-page".to_string()],
        };
        protocol
            .style_closures
            .insert("index.html".to_string(), closure.clone());
        protocol
            .style_closures
            .insert("my-page".to_string(), closure);
    }

    fn response_json(response: ServeResponse) -> serde_json::Value {
        match response {
            ServeResponse::Json(json) => {
                serde_json::from_str(&json).expect("partial response should be valid JSON")
            }
            ServeResponse::Html(_) => panic!("expected JSON partial response"),
        }
    }

    #[test]
    fn serve_request_json_partial_carries_module_component_styles() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-page")],
                contains_boundary: false,
            },
        );
        fragments.insert(
            "my-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>page</p>")],
                contains_boundary: false,
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        protocol.set_css_strategy(webui_protocol::CssStrategy::Module);
        let component = protocol
            .components
            .entry("my-page".to_string())
            .or_default();
        component.template_json = r#"{"h":"<p>page</p>"}"#.to_string();
        component.css = ".page{color:red}".to_string();
        add_page_style_closures(&mut protocol);
        let protocol = Protocol::new(protocol);

        let handler = WebUIHandler::new();
        let request = ServeRequest {
            path: "/",
            accept_json: true,
            inventory_hex: "",
        };

        let response = serve_request(&protocol, &handler, json!({}), "index.html", &request)
            .expect("partial response should succeed");

        let json = response_json(response);

        assert_eq!(
            json["componentStyles"]["resources"]["my-page"],
            json!({
                "kind": "module",
                "specifier": "my-page",
                "css": ".page{color:red}"
            })
        );
        // templates must not contain any style tags
        assert!(
            !json["templates"]["my-page"]["h"]
                .as_str()
                .unwrap_or_default()
                .contains("<style"),
            "template metadata should not contain module style tags"
        );
    }

    #[test]
    fn serve_request_link_strategy_returns_component_styles() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-page")],
                contains_boundary: false,
            },
        );
        fragments.insert(
            "my-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>page</p>")],
                contains_boundary: false,
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        let comp = protocol
            .components
            .entry("my-page".to_string())
            .or_default();
        comp.template_json = r#"{"h":"<p>page</p>"}"#.to_string();
        comp.css_href = "my-page.css".to_string();
        add_page_style_closures(&mut protocol);
        let protocol = Protocol::new(protocol);

        let handler = WebUIHandler::new();
        let request = ServeRequest {
            path: "/",
            accept_json: true,
            inventory_hex: "",
        };

        let response = serve_request(&protocol, &handler, json!({}), "index.html", &request)
            .expect("partial response should succeed");

        let json = response_json(response);

        assert_eq!(
            json["componentStyles"]["resources"]["my-page"],
            json!({"kind": "link", "href": "my-page.css"})
        );
        assert!(
            json["templates"].as_object().is_some_and(|a| a.len() == 1),
            "Link strategy should still return templates"
        );
    }

    #[test]
    fn serve_request_style_strategy_returns_component_styles() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-page")],
                contains_boundary: false,
            },
        );
        fragments.insert(
            "my-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>page</p>")],
                contains_boundary: false,
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        protocol.set_css_strategy(CssStrategy::Style);
        let comp = protocol
            .components
            .entry("my-page".to_string())
            .or_default();
        comp.template_json = r#"{"h":"<p/>"}"#.to_string();
        comp.css = ".p{color:red}".to_string();
        add_page_style_closures(&mut protocol);
        let protocol = Protocol::new(protocol);

        let handler = WebUIHandler::new();
        let request = ServeRequest {
            path: "/",
            accept_json: true,
            inventory_hex: "",
        };

        let response = serve_request(&protocol, &handler, json!({}), "index.html", &request)
            .expect("partial response should succeed");

        let json = response_json(response);

        assert_eq!(
            json["componentStyles"]["resources"]["my-page"],
            json!({"kind": "style", "css": ".p{color:red}"})
        );
        assert!(
            json["templates"].as_object().is_some_and(|a| a.len() == 1),
            "Style strategy should still return templates"
        );
    }
}
