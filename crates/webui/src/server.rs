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
        // JSON partial response for client-side navigation
        let partial = route_handler::render_partial(
            protocol,
            data,
            entry,
            request.path,
            request.inventory_hex,
        );

        let (needed, new_inv) = route_handler::get_needed_components_for_request(
            protocol,
            entry,
            request.path,
            request.inventory_hex,
        );

        let templates: Vec<serde_json::Value> = needed
            .iter()
            .filter_map(|name| {
                protocol
                    .components
                    .get(name)
                    .map(|c| c.template.as_str())
                    .filter(|s| !s.is_empty())
            })
            .map(|t| serde_json::Value::String(t.to_string()))
            .collect();

        let mut resp = match partial {
            serde_json::Value::Object(m) => m,
            _ => serde_json::Map::new(),
        };
        resp.insert("templates".into(), serde_json::Value::Array(templates));
        resp.insert("inventory".into(), serde_json::Value::String(new_inv));

        Ok(ServeResponse::Json(serde_json::Value::Object(resp)))
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
