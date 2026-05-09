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
use webui::{build, BuildOptions, CssStrategy, DomStrategy, Plugin, WebUIHandler, WebUIProtocol};
use webui_handler::plugin::webui::WebUIHydrationPlugin;
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
            dom: DomStrategy::Shadow,
            plugin: Some(Plugin::WebUI),
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
        route_handler::collect_nested_route_params(
            &self.protocol,
            &self.entry,
            route_path,
            &mut webui_handler::route_matcher::CompiledRouteCache::new(),
        )
    }

    pub fn render_html(&self, route_path: &str, state: &Value, nonce: &str) -> Result<String> {
        let mut writer = MemoryWriter::with_capacity(16_384);
        let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
        let opts = RenderOptions::new(&self.entry, route_path).with_nonce(nonce);
        handler
            .handle(&self.protocol, state, &opts, &mut writer)
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
        let mut index = route_handler::ProtocolIndex::new(&self.protocol);
        let mut partial = route_handler::render_partial(
            &self.protocol,
            &self.entry,
            route_path,
            inventory_hex,
            &mut index,
        )
        .unwrap_or_else(|e| serde_json::json!({"error": format!("render_partial failed: {e}")}));
        if let Some(obj) = partial.as_object_mut() {
            // Broadcast the same shared state object to every chain entry —
            // each component picks only its own @observable keys. JSON
            // serializes the duplicates once and the runtime carries N
            // references to the same object, so memory cost is one pointer
            // slot per entry.
            let chain_len = obj
                .get("chain")
                .and_then(|c| c.as_array())
                .map_or(1, std::vec::Vec::len);
            let mut states = Vec::with_capacity(chain_len);
            for _ in 0..chain_len.saturating_sub(1) {
                states.push(state.clone());
            }
            states.push(state);
            obj.insert("states".into(), Value::Array(states));
        }
        partial
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

/// Returns `true` if the filename contains an esbuild content hash, making it
/// safe for immutable caching. Esbuild produces `chunk-{HASH}.js` for shared
/// chunks and `{name}-{HASH}.js` for page-specific entry points.
fn is_content_hashed(relative: &str) -> bool {
    let name = relative.rsplit('/').next().unwrap_or(relative);
    if !name.ends_with(".js") && !name.ends_with(".js.map") {
        return false;
    }
    // Skip bare entry points like `index.js`
    let stem = name.split('.').next().unwrap_or("");
    // Content-hashed files always have a hyphenated 8-char uppercase hash suffix
    // e.g. "chunk-3QJD3BDH" or "mp-page-home-UFH4TZ7P"
    stem.rsplit('-').next().is_some_and(|hash| {
        hash.len() == 8
            && hash
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    })
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

    #[test]
    fn content_hashed_js_chunks_detected() {
        assert!(super::is_content_hashed("chunk-3QJD3BDH.js"));
        assert!(super::is_content_hashed("chunk-YXUYDP2R.js"));
        assert!(super::is_content_hashed("mp-page-home-UFH4TZ7P.js"));
        assert!(super::is_content_hashed("mp-page-product-3BEKONPP.js"));
    }

    #[test]
    fn content_hashed_sourcemaps_detected() {
        assert!(super::is_content_hashed("chunk-3QJD3BDH.js.map"));
        assert!(super::is_content_hashed("mp-page-home-UFH4TZ7P.js.map"));
    }

    #[test]
    fn unhashed_files_not_detected() {
        assert!(!super::is_content_hashed("index.js"));
        assert!(!super::is_content_hashed("index.js.map"));
        assert!(!super::is_content_hashed("mp-app.css"));
        assert!(!super::is_content_hashed("mp-page-home.css"));
    }
}
