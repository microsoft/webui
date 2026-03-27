// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use actix_web::web::Bytes;
use actix_web::{HttpRequest, HttpResponse};
use anyhow::{Context, Result};
use mime_guess::from_path;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use webui::{build, BuildOptions, CssStrategy, WebUIHandler, WebUIProtocol};
use webui_handler::plugin::fast::FastHydrationPlugin;
use webui_handler::route_handler;
use webui_handler::{RenderOptions, ResponseWriter};

#[derive(Clone)]
pub struct FrontendRuntime {
    css_files: HashMap<String, Bytes>,
    asset_files: HashMap<String, CachedAsset>,
    entry: String,
    protocol: WebUIProtocol,
}

#[derive(Clone)]
struct CachedAsset {
    content_type: String,
    body: Bytes,
}

impl FrontendRuntime {
    pub fn load(app_root: &Path, css: CssStrategy) -> Result<Self> {
        let app_dir = app_root.join("src");
        let assets_dir = canonicalize_dir(&app_root.join("dist"));
        let build_result = build(BuildOptions {
            app_dir,
            entry: "index.html".to_string(),
            css,
            plugin: Some("fast".to_string()),
            components: Vec::new(),
        })
        .with_context(|| "Failed to build the commerce WebUI protocol")?;

        Ok(Self {
            css_files: build_result
                .css_files
                .into_iter()
                .map(|(path, css)| (path, Bytes::from(css)))
                .collect(),
            asset_files: load_cached_assets(&assets_dir)?,
            entry: "index.html".to_string(),
            protocol: build_result.protocol,
        })
    }

    /// Collect route params from the nested route tree for a given path.
    pub fn collect_route_params(&self, route_path: &str) -> HashMap<String, String> {
        route_handler::collect_nested_route_params(&self.protocol, &self.entry, route_path)
    }

    pub fn render_html(&self, route_path: &str, state: &Value) -> Result<String> {
        let mut writer = MemoryWriter::with_capacity(16_384);
        let handler = WebUIHandler::with_plugin(|| Box::new(FastHydrationPlugin::new()));
        handler
            .handle(
                &self.protocol,
                state,
                &RenderOptions::new(&self.entry, route_path),
                &mut writer,
            )
            .with_context(|| format!("Failed to render HTML for {route_path}"))?;
        Ok(writer.buf)
    }

    #[must_use]
    pub fn render_partial(
        &self,
        route_path: &str,
        _request_path: &str,
        inventory_hex: &str,
        state: Value,
    ) -> Value {
        route_handler::render_partial(
            &self.protocol,
            state,
            &self.entry,
            route_path,
            inventory_hex,
        )
    }

    #[must_use]
    pub fn serve_asset(&self, relative: &str) -> Option<HttpResponse> {
        if let Some(css) = self.css_files.get(relative) {
            return Some(
                HttpResponse::Ok()
                    .content_type("text/css; charset=utf-8")
                    .body(css.clone()),
            );
        }

        self.asset_files.get(relative).map(|asset| {
            HttpResponse::Ok()
                .content_type(asset.content_type.as_str())
                .body(asset.body.clone())
        })
    }
}

fn canonicalize_dir(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn load_cached_assets(assets_dir: &Path) -> Result<HashMap<String, CachedAsset>> {
    let mut assets = HashMap::new();
    if !assets_dir.is_dir() {
        return Ok(assets);
    }

    let mut pending = vec![assets_dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir)
            .with_context(|| format!("Failed to read asset directory {}", dir.display()))?
        {
            let entry = entry.with_context(|| {
                format!("Failed to read an asset entry under {}", dir.display())
            })?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .with_context(|| format!("Failed to inspect asset {}", path.display()))?;

            if file_type.is_dir() {
                pending.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }

            let relative = path
                .strip_prefix(assets_dir)
                .with_context(|| format!("Failed to relativize asset {}", path.display()))?;
            let key = relative.to_string_lossy().replace('\\', "/");
            let body = fs::read(&path)
                .with_context(|| format!("Failed to read cached asset {}", path.display()))?;
            let content_type = from_path(&path).first_or_octet_stream().to_string();

            assets.insert(
                key,
                CachedAsset {
                    content_type,
                    body: Bytes::from(body),
                },
            );
        }
    }

    Ok(assets)
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

struct MemoryWriter {
    buf: String,
}

impl MemoryWriter {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            buf: String::with_capacity(capacity),
        }
    }
}

impl ResponseWriter for MemoryWriter {
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
    use super::{canonicalize_dir, load_cached_assets};
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
            load_cached_assets(&canonicalize_dir(&root)).unwrap_or_else(|error| panic!("{error}"));
        fs::remove_file(&asset_path).unwrap_or_else(|error| panic!("{error}"));

        let asset = cache
            .get("nested/index.js")
            .unwrap_or_else(|| panic!("expected nested/index.js to be cached"));
        assert!(asset.content_type.contains("javascript"));
        assert_eq!(asset.body.as_ref(), b"console.log('cached');");

        let _ = fs::remove_dir_all(&root);
    }
}
