// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Renders the demo shell — itself a WebUI app at `examples/demo/src/` —
//! using the in-process `webui::build` + `WebUIHandler` pipeline.
//!
//! The shell's protocol is compiled once at startup. Each request renders
//! the cached protocol against a fresh JSON state derived from the
//! discovered app registry. Static client assets (`dist/index.js`) are
//! served from `<shell_dir>/dist/`.

use actix_web::{web, HttpRequest, HttpResponse};
use std::path::PathBuf;
use std::sync::Arc;

use webui::{build, BuildOptions, CssStrategy, DomStrategy, Plugin};
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::{RenderOptions, ResponseWriter, WebUIHandler};
use webui_protocol::WebUIProtocol;

use crate::registry::AppEntry;

/// Shared state for the shell renderer: the compiled protocol and the
/// directory containing client-side assets (`dist/`).
pub(crate) struct ShellState {
    pub(crate) protocol: WebUIProtocol,
    pub(crate) assets_dir: PathBuf,
}

impl ShellState {
    /// Compile the shell app's protocol from `<shell_dir>/src/`.
    pub(crate) fn build(shell_dir: &std::path::Path) -> anyhow::Result<Arc<Self>> {
        let src_dir = shell_dir.join("src");
        let assets_dir = shell_dir.join("dist");

        if !src_dir.is_dir() {
            anyhow::bail!(
                "Shell source directory not found: {}. \
                 Pass --shell-dir pointing at examples/demo/.",
                src_dir.display()
            );
        }

        log::info!("Compiling shell protocol from {}", src_dir.display());
        let result = build(BuildOptions {
            app_dir: src_dir,
            entry: "index.html".to_string(),
            css: CssStrategy::Link,
            dom: DomStrategy::Shadow,
            plugin: Some(Plugin::WebUI),
            components: Vec::new(),
            entry_point: None,
        })
        .map_err(|e| anyhow::anyhow!("Failed to build shell protocol: {e}"))?;

        log::info!(
            "Shell protocol compiled: {} fragment(s), {} byte(s)",
            result.stats.fragment_count,
            result.stats.protocol_size_bytes
        );

        Ok(Arc::new(Self {
            protocol: result.protocol,
            assets_dir,
        }))
    }
}

/// Build the per-request render state from the discovered app registry.
fn build_state(apps: &[AppEntry], current_index: usize) -> serde_json::Value {
    let total = apps.len();
    let app_array: Vec<serde_json::Value> = apps
        .iter()
        .map(|a| {
            serde_json::json!({
                "slug": a.slug,
                "name": a.name,
                "description": a.description,
                "backend": a.backend,
                "sourceUrl": a.source_url(),
                "iframeUrl": format!("/{}/", a.slug),
            })
        })
        .collect();

    let current = apps.get(current_index).expect("current_index in range");

    serde_json::json!({
        "basePath": "/_shell/",
        "apps": app_array,
        "currentApp": {
            "slug": current.slug,
            "name": current.name,
            "description": current.description,
            "backend": current.backend,
            "sourceUrl": current.source_url(),
            "iframeUrl": format!("/{}/", current.slug),
        },
        "totalApps": total,
        "currentDisplay": current_index + 1,
    })
}

struct StringWriter {
    buf: String,
}

impl ResponseWriter for StringWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.buf.push_str(content);
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

/// Serves the shell page at `/`. Picks the initial current app via the
/// `?app=<slug>` query parameter when present.
pub(crate) async fn shell_page(
    req: HttpRequest,
    shell: web::Data<Arc<ShellState>>,
    apps: web::Data<Vec<AppEntry>>,
) -> HttpResponse {
    if apps.is_empty() {
        return HttpResponse::ServiceUnavailable().body("No apps registered");
    }

    let current_index = req
        .uri()
        .query()
        .and_then(|q| {
            q.split('&')
                .find_map(|kv| kv.strip_prefix("app="))
                .map(|s| s.to_string())
        })
        .and_then(|slug| apps.iter().position(|a| a.slug == slug))
        .unwrap_or(0);

    let state = build_state(&apps, current_index);

    let mut writer = StringWriter {
        buf: String::with_capacity(8 * 1024),
    };
    let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
    let opts = RenderOptions::new("index.html", "/");

    if let Err(e) = handler.handle(&shell.protocol, &state, &opts, &mut writer) {
        log::error!("Shell render failed: {e}");
        return HttpResponse::InternalServerError().body(format!("Shell render failed: {e}"));
    }

    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .insert_header(("Cache-Control", "no-cache"))
        .body(writer.buf)
}

/// Serves a static client asset from `<shell_dir>/dist/{tail}`.
///
/// Mounted at `/_shell/{tail:.*}` to avoid colliding with the `/{slug}/`
/// proxy namespace. Path traversal attempts (`..`) are rejected.
pub(crate) async fn shell_asset(
    path: web::Path<String>,
    shell: web::Data<Arc<ShellState>>,
) -> HttpResponse {
    let tail = path.into_inner();

    // Reject path traversal attempts.
    if tail.split('/').any(|seg| seg == ".." || seg.is_empty()) {
        return HttpResponse::NotFound().finish();
    }

    let asset_path = shell.assets_dir.join(&tail);
    if !asset_path.is_file() {
        return HttpResponse::NotFound().body(format!("Asset not found: {tail}"));
    }

    match std::fs::read(&asset_path) {
        Ok(bytes) => {
            let mime = mime_guess::from_path(&asset_path).first_or_octet_stream();
            HttpResponse::Ok()
                .content_type(mime.as_ref())
                .insert_header(("Cache-Control", "public, max-age=300"))
                .body(bytes)
        }
        Err(e) => {
            log::error!("Failed to read shell asset {}: {e}", asset_path.display());
            HttpResponse::InternalServerError().finish()
        }
    }
}
