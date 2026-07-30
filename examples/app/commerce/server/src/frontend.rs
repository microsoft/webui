// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use crate::dist_assets::{is_content_hashed, load_dist_assets, CachedAsset};
use actix_web::web::Bytes;
use actix_web::{HttpRequest, HttpResponse};
use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use webui::{build, BuildOptions, CssStrategy, Plugin, Protocol, WebUIHandler};
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::route_handler;
use webui_handler::{RenderOptions, ResponseWriter};

#[derive(Clone)]
pub struct FrontendRuntime {
    css_files: HashMap<String, Bytes>,
    asset_files: HashMap<String, CachedAsset>,
    entry: String,
    protocol: Arc<Protocol>,
}

impl FrontendRuntime {
    pub fn load(app_root: &Path, css: CssStrategy) -> Result<Self> {
        Self::load_with_projection(app_root, css, true)
    }

    #[cfg(test)]
    pub(crate) fn load_for_tests(app_root: &Path, css: CssStrategy) -> Result<Self> {
        Self::load_with_projection(app_root, css, false)
    }

    fn load_with_projection(
        app_root: &Path,
        css: CssStrategy,
        use_projection_manifest: bool,
    ) -> Result<Self> {
        let app_dir = app_root.join("src");
        let assets_dir = canonicalize_dir(&app_root.join("dist"));
        // Production consumes the client build's manifest when present. Unit
        // tests deliberately build from source without generated client
        // artifacts, so their result cannot depend on stale local dist files.
        let manifest_path = app_root.join("dist").join("webui-projection.json");
        let projection_manifests = if use_projection_manifest && manifest_path.is_file() {
            vec![webui::ProjectionManifestSource::Path(manifest_path)]
        } else {
            Vec::new()
        };
        let build_result = build(BuildOptions {
            app_dir,
            entry: "index.html".to_string(),
            css,
            plugin: Some(Plugin::WebUI),
            projection_manifests,
            ..BuildOptions::default()
        })
        .with_context(|| "Failed to build the commerce WebUI protocol")?;

        Ok(Self {
            css_files: build_result
                .css_files
                .into_iter()
                .map(|(path, css)| (path, Bytes::from(css)))
                .collect(),
            asset_files: load_dist_assets(&assets_dir)?,
            entry: "index.html".to_string(),
            protocol: Arc::new(Protocol::new(build_result.protocol)),
        })
    }

    /// Collect route params from the nested route tree for a given path.
    pub fn collect_route_params(&self, route_path: &str) -> HashMap<String, String> {
        route_handler::collect_nested_route_params(&self.protocol, &self.entry, route_path)
    }

    /// Stream the SSR HTML for `route_path` into `writer`. Used by the
    /// streaming response path to avoid materialising the full HTML in
    /// memory before sending the first byte to the client. The writer is
    /// typically a [`webui::streaming::StreamingWriter`].
    ///
    /// `head_inject` (optional) is HTML emitted at the structural
    /// `</head>` close — used here for per-request `<link
    /// rel="preload">` image hints. Inserted by the handler at the
    /// `head_end` signal boundary, so it cannot mis-fire on `</head>`
    /// literals appearing in HTML comments / `srcdoc` / scripts.
    #[allow(clippy::too_many_arguments)]
    pub fn render_html_to<W: ResponseWriter>(
        &self,
        route_path: &str,
        state: &Value,
        nonce: &str,
        head_inject: &str,
        writer: &mut W,
    ) -> Result<()> {
        let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
        let opts = RenderOptions::new(&self.entry, route_path)
            .with_nonce(nonce)
            .with_head_inject(head_inject);
        handler
            .render(&self.protocol, state, &opts, writer)
            .with_context(|| format!("Failed to render HTML for {route_path}"))?;
        Ok(())
    }

    #[must_use]
    pub fn render_partial(
        &self,
        route_path: &str,
        _request_path: &str,
        inventory_hex: &str,
        state: Value,
    ) -> Value {
        let state_json = match serde_json::to_string(&state) {
            Ok(value) => value,
            Err(error) => {
                return serde_json::json!({
                    "error": format!("state serialization failed: {error}")
                });
            }
        };
        match self
            .protocol
            .render_partial(&state_json, &self.entry, route_path, inventory_hex)
        {
            Ok(json) => serde_json::from_str(&json).unwrap_or_else(|error| {
                serde_json::json!({
                    "error": format!("partial response decoding failed: {error}")
                })
            }),
            Err(error) => {
                serde_json::json!({"error": format!("render_partial failed: {error}")})
            }
        }
    }

    #[must_use]
    pub fn serve_asset(&self, relative: &str) -> Option<HttpResponse> {
        if let Some(css) = self.css_files.get(relative) {
            // CSS filenames are not content-hashed, so use a moderate
            // max-age with revalidation instead of immutable.
            return Some(
                HttpResponse::Ok()
                    .content_type("text/css; charset=utf-8")
                    .insert_header(("Cache-Control", "public, max-age=86400, must-revalidate"))
                    .body(css.clone()),
            );
        }

        self.asset_files.get(relative).map(|asset| {
            let cache = if is_content_hashed(relative) {
                "public, max-age=31536000, immutable"
            } else {
                "public, max-age=86400"
            };
            HttpResponse::Ok()
                .content_type(asset.content_type.as_str())
                .insert_header(("Cache-Control", cache))
                .body(asset.body.clone())
        })
    }
}

fn canonicalize_dir(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

pub fn wants_json(req: &HttpRequest) -> bool {
    req.headers()
        .get("accept")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("application/json"))
}

#[must_use]
pub fn route_path(req: &HttpRequest) -> &str {
    req.path()
}

#[must_use]
pub fn request_path(req: &HttpRequest) -> String {
    req.uri().path_and_query().map_or_else(
        || req.path().to_string(),
        |value| value.as_str().to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::canonicalize_dir;
    use crate::dist_assets::load_dist_assets;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn cached_assets_survive_source_file_removal() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let root = std::env::temp_dir().join(format!(
            "webui-commerce-asset-cache-{}-{unique}",
            std::process::id()
        ));
        let nested = root.join("nested");
        let asset_path = nested.join("index.js");
        fs::create_dir_all(&nested).unwrap_or_else(|error| panic!("{error}"));
        fs::write(&asset_path, "console.log('cached');").unwrap_or_else(|error| panic!("{error}"));

        let cache =
            load_dist_assets(&canonicalize_dir(&root)).unwrap_or_else(|error| panic!("{error}"));
        fs::remove_file(&asset_path).unwrap_or_else(|error| panic!("{error}"));

        let asset = cache
            .get("nested/index.js")
            .unwrap_or_else(|| panic!("expected nested/index.js to be cached"));
        assert!(asset.content_type.contains("javascript"));
        assert_eq!(asset.body.as_ref(), b"console.log('cached');");

        let _ = fs::remove_dir_all(&root);
    }
}
