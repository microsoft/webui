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
//! use webui::{
//!     server::{serve_request, ServeRequest, ServeResponse},
//!     RenderOptions,
//! };
//!
//! let options = RenderOptions::new("index.html", "/email/thread-5")
//!     .with_nonce("request-csp-nonce");
//! let request = ServeRequest::new(options, false, "");
//!
//! match serve_request(&protocol, &handler, state, &request) {
//!     Ok(ServeResponse::Html(html)) => { /* serve HTML */ }
//!     Ok(ServeResponse::Json(json)) => { /* serve JSON */ }
//!     Err(e) => { /* handle error */ }
//! }
//! ```

use crate::{Protocol, WebUIHandler};
use webui_handler::route_handler;
use webui_handler::RenderOptions;

webui_handler::define_string_response_writer!(MemWriter, buf);

/// A server request to be handled by [`serve_request`].
pub struct ServeRequest<'a> {
    render_options: RenderOptions<'a>,
    accept_json: bool,
    inventory_hex: &'a str,
}

impl<'a> ServeRequest<'a> {
    /// Create a server request from the complete per-render configuration.
    ///
    /// Set `accept_json` when the request accepts a partial-navigation response.
    /// `inventory_hex` is the client's `X-WebUI-Inventory` value, or an empty
    /// string when the header is absent.
    #[must_use]
    #[inline]
    pub fn new(
        render_options: RenderOptions<'a>,
        accept_json: bool,
        inventory_hex: &'a str,
    ) -> Self {
        Self {
            render_options,
            accept_json,
            inventory_hex,
        }
    }
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
/// - `request` — The incoming request details and complete [`RenderOptions`]
///
/// # Route Parameters
/// Route parameters (`:param` in route paths) are automatically extracted
/// and injected into the state object.
///
/// # Content Security Policy
/// Set the response nonce with [`RenderOptions::with_nonce`]. Full-document
/// rendering forwards the options unchanged so every generated inline script
/// and the `webui-nonce` metadata receive the request nonce.
pub fn serve_request(
    protocol: &Protocol,
    handler: &WebUIHandler,
    state: serde_json::Value,
    request: &ServeRequest<'_>,
) -> Result<ServeResponse, String> {
    let options = &request.render_options;

    // Extract route params and inject into state
    let params = route_handler::collect_nested_route_params(
        protocol,
        options.entry_id,
        options.request_path,
    );
    let mut data = state;
    if let Some(map) = data.as_object_mut() {
        for (key, value) in params {
            map.insert(key, serde_json::Value::String(value));
        }
    }

    if request.accept_json {
        // JSON partial response for client-side navigation.
        let partial = protocol
            .render_partial(
                data,
                options.entry_id,
                options.request_path,
                request.inventory_hex,
            )
            .map_err(|e| format!("render_partial failed: {e}"))?;
        Ok(ServeResponse::Json(partial))
    } else {
        // Full HTML SSR — handler emits the consolidated window.__webui
        // script block (state, chain, inventory, templates) automatically.
        let mut writer = MemWriter::with_capacity(131_072);
        handler
            .render(protocol, &data, options, &mut writer)
            .map_err(|e| format!("render failed: {e}"))?;

        Ok(ServeResponse::Html(writer.buf))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    use webui_handler::plugin::webui::WebUIHydrationPlugin;
    use webui_protocol::{
        ComponentData, ComponentStyleClosure, CssStrategy, FragmentList, WebUIFragment,
        WebUIProtocol,
    };

    fn structural_fragment(value: &str) -> WebUIFragment {
        let mut signal = String::with_capacity("}}}webui:".len() + value.len());
        signal.push_str("}}}webui:");
        signal.push_str(value);
        WebUIFragment::signal(signal, true)
    }

    fn add_page_style_closures(protocol: &mut WebUIProtocol) {
        let closure = ComponentStyleClosure {
            component_tags: vec!["my-page".to_string()],
            style_chunks: Vec::new(),
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

    fn full_document_protocol() -> Protocol {
        let fragments = HashMap::from([
            (
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<!doctype html><html><head>"),
                        structural_fragment("head_end"),
                        WebUIFragment::raw("</head><body>"),
                        WebUIFragment::component("my-page"),
                        structural_fragment("body_end"),
                        WebUIFragment::raw("</body></html>"),
                    ],
                    contains_boundary: false,
                },
            ),
            (
                "my-page".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("<main>ready</main>")],
                    contains_boundary: false,
                },
            ),
        ]);
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.components.insert(
            "my-page".to_string(),
            ComponentData {
                template_json: r#"{"h":"<main>ready</main>","th":1}"#.to_string(),
                template_functions: "[function(){return true}]".to_string(),
                ..Default::default()
            },
        );
        Protocol::new(protocol)
    }

    #[test]
    fn serve_request_html_forwards_complete_render_options() {
        let protocol = full_document_protocol();
        let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
        let request = ServeRequest::new(
            RenderOptions::new("index.html", "/")
                .with_nonce("request-nonce")
                .with_head_inject("<meta name=\"host-head\">")
                .with_body_inject("<script src=\"host.js\"></script>"),
            false,
            "",
        );

        let response = serve_request(&protocol, &handler, json!({}), &request)
            .expect("full-document response should succeed");
        let html = match response {
            ServeResponse::Html(html) => html,
            ServeResponse::Json(_) => panic!("expected full-document response"),
        };

        let mut direct = MemWriter::with_capacity(131_072);
        handler
            .render(&protocol, &json!({}), &request.render_options, &mut direct)
            .expect("direct render should succeed");

        assert_eq!(html, direct.buf);
        assert!(html.contains(r#"<meta name="webui-nonce" content="request-nonce">"#));
        assert!(html.contains(r#"<script nonce="request-nonce">(function(){var w=window.__webui"#));
        assert!(html.contains(r#"<meta name="host-head">"#));
        assert!(html.contains(r#"<script src="host.js"></script>"#));
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
        let request = ServeRequest::new(RenderOptions::new("index.html", "/"), true, "");

        let response = serve_request(&protocol, &handler, json!({}), &request)
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
        let request = ServeRequest::new(RenderOptions::new("index.html", "/"), true, "");

        let response = serve_request(&protocol, &handler, json!({}), &request)
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
        let request = ServeRequest::new(RenderOptions::new("index.html", "/"), true, "");

        let response = serve_request(&protocol, &handler, json!({}), &request)
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
