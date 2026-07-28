// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Bounded, load-once in-memory cache of this app's `dist/` build output.
//!
//! `esbuild --splitting` (see `../../build-client.mjs`) produces one or more
//! content-hashed shared chunks (`chunk-HASH.js`) alongside the stable
//! `index.js` entry — the exact set of filenames is not known until after
//! the client build runs, so the server cannot hard-code a fixed asset
//! list or a fixed set of routes. Every file under `dist/` is read once at
//! startup into this map; each request is served by an exact key lookup,
//! never a per-request filesystem read, so an arbitrary request path can
//! only ever resolve to a file that was actually present in `dist/` at
//! startup (path traversal has no filesystem to traverse). Mirrors
//! `examples/app/commerce/server/src/frontend.rs`'s asset cache.

use actix_web::HttpResponse;
use anyhow::{Context, Result};
use bytes::Bytes;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub(crate) struct CachedAsset {
    content_type: String,
    body: Bytes,
}

/// Read every file under `dist_dir` once, keyed by its path relative to
/// `dist_dir` with forward slashes (`"chunk-NKNSLYVV.js"`,
/// `"nested/asset.js"`). Returns an empty map (not an error) if `dist_dir`
/// doesn't exist yet, so the caller can report one clear "run
/// `pnpm build:client` first" error instead of a directory-read failure.
pub(crate) fn load_dist_assets(dist_dir: &Path) -> Result<HashMap<String, CachedAsset>> {
    let mut assets = HashMap::new();
    if !dist_dir.is_dir() {
        return Ok(assets);
    }

    // Iterative traversal (not recursive) so directory depth cannot grow
    // the call stack; `esbuild --outdir` output is flat in practice, but
    // this stays correct if that ever changes.
    let mut pending = vec![dist_dir.to_path_buf()];
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
                .strip_prefix(dist_dir)
                .with_context(|| format!("Failed to relativize asset {}", path.display()))?;
            let key = relative.to_string_lossy().replace('\\', "/");
            let body = fs::read(&path)
                .with_context(|| format!("Failed to read asset {}", path.display()))?;
            let content_type = mime_guess::from_path(&path)
                .first_or_octet_stream()
                .to_string();

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

/// Add the component CSS files the WebUI build produced to `assets`.
///
/// Component CSS never reaches `dist/` — the client build only emits
/// JavaScript, while `build()` returns component stylesheets in memory as
/// [`BuildResult::css_files`](microsoft_webui::BuildResult::css_files).
/// Without this the `link` and `module` CSS strategies emit
/// `<link rel="stylesheet" href="feed-item.css">` into every shadow root and
/// a matching `<link rel="preload">` in `<head>`, and every one of those
/// requests 404s, so the page renders unstyled.
///
/// Filenames are content-stable (`<tag>.css`), not content-hashed, so
/// [`asset_response`] serves them with `no-cache` and they revalidate.
pub(crate) fn insert_generated_css(
    assets: &mut HashMap<String, CachedAsset>,
    css_files: Vec<(String, String)>,
) {
    for (name, content) in css_files {
        assets.insert(
            name,
            CachedAsset {
                content_type: "text/css; charset=utf-8".to_owned(),
                body: Bytes::from(content),
            },
        );
    }
}

/// Look up `relative` by exact key and build the HTTP response, or `None`
/// if no such asset was loaded at startup.
#[must_use]
pub(crate) fn asset_response(
    assets: &HashMap<String, CachedAsset>,
    relative: &str,
) -> Option<HttpResponse> {
    assets.get(relative).map(|asset| {
        // Esbuild's content-hashed chunk filenames change whenever their
        // content changes, so they are safe to cache immutably; the
        // stable `index.js` entry point is not, and must revalidate.
        let cache_control = if is_content_hashed(relative) {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        HttpResponse::Ok()
            .content_type(asset.content_type.as_str())
            .insert_header(("Cache-Control", cache_control))
            .body(asset.body.clone())
    })
}

/// `true` for esbuild's content-hashed shared chunks (`chunk-HASH.js` /
/// `chunk-HASH.js.map`). Mirrors
/// `examples/app/commerce/server/src/frontend.rs::is_content_hashed`.
fn is_content_hashed(relative: &str) -> bool {
    let name = relative.rsplit('/').next().unwrap_or(relative);
    if !name.ends_with(".js") && !name.ends_with(".js.map") {
        return false;
    }
    let stem = name.split('.').next().unwrap_or("");
    stem.rsplit('-').next().is_some_and(|hash| {
        hash.len() == 8
            && hash
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    })
}

#[cfg(test)]
mod tests {
    use super::{asset_response, insert_generated_css, is_content_hashed, load_dist_assets};
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        std::env::temp_dir().join(format!(
            "webui-streaming-assets-{label}-{}-{unique}",
            std::process::id()
        ))
    }

    #[test]
    fn loads_every_file_in_a_flat_dist_directory() {
        let dir = temp_dir("flat");
        fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        fs::write(dir.join("index.js"), "console.log('entry');").unwrap_or_else(|e| panic!("{e}"));
        fs::write(dir.join("chunk-NKNSLYVV.js"), "export const x = 1;")
            .unwrap_or_else(|e| panic!("{e}"));

        let assets = load_dist_assets(&dir).unwrap_or_else(|e| panic!("{e}"));
        assert!(assets.contains_key("index.js"));
        assert!(assets.contains_key("chunk-NKNSLYVV.js"));
        assert_eq!(assets.len(), 2);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_dist_directory_yields_an_empty_map_not_an_error() {
        let dir = temp_dir("missing");
        let assets = load_dist_assets(&dir).unwrap_or_else(|e| panic!("{e}"));
        assert!(assets.is_empty());
    }

    #[test]
    fn asset_response_serves_known_keys_with_correct_content_type() {
        let dir = temp_dir("response");
        fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        fs::write(dir.join("index.js"), "console.log('entry');").unwrap_or_else(|e| panic!("{e}"));
        let assets = load_dist_assets(&dir).unwrap_or_else(|e| panic!("{e}"));

        let response =
            asset_response(&assets, "index.js").unwrap_or_else(|| panic!("expected index.js"));
        let content_type = response
            .headers()
            .get(actix_web::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        assert!(content_type.contains("javascript"));
        assert_eq!(
            response
                .headers()
                .get(actix_web::http::header::CACHE_CONTROL)
                .and_then(|value| value.to_str().ok()),
            Some("no-cache")
        );

        assert!(asset_response(&assets, "missing.js").is_none());
        // Path-traversal attempts never touch the filesystem — they simply
        // fail the exact-key lookup like any other unknown key.
        assert!(asset_response(&assets, "../../etc/passwd").is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    /// The `link` and `module` CSS strategies emit
    /// `<link rel="stylesheet" href="feed-item.css">` into every shadow root
    /// plus a `<link rel="preload">` in `<head>`. Those files exist only in
    /// the build result, so without this merge every one of them 404s and the
    /// page renders unstyled.
    #[test]
    fn generated_component_css_is_served_as_css() {
        let mut assets = HashMap::new();
        insert_generated_css(
            &mut assets,
            vec![
                (
                    "feed-item.css".to_owned(),
                    ".feed-item{color:red}".to_owned(),
                ),
                (
                    "message-composer.css".to_owned(),
                    ".composer{display:flex}".to_owned(),
                ),
            ],
        );

        let response = asset_response(&assets, "feed-item.css")
            .unwrap_or_else(|| panic!("expected feed-item.css"));
        let content_type = response
            .headers()
            .get(actix_web::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        assert!(content_type.starts_with("text/css"), "{content_type}");
        // Stable `<tag>.css` names are not content-hashed, so they must
        // revalidate rather than be cached immutably.
        assert!(!is_content_hashed("feed-item.css"));
        assert_eq!(
            response
                .headers()
                .get(actix_web::http::header::CACHE_CONTROL)
                .and_then(|value| value.to_str().ok()),
            Some("no-cache")
        );
        assert!(asset_response(&assets, "message-composer.css").is_some());
    }

    #[test]
    fn generated_css_does_not_evict_dist_assets() {
        let dir = temp_dir("merge");
        fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        fs::write(dir.join("index.js"), "console.log('entry');").unwrap_or_else(|e| panic!("{e}"));
        let mut assets = load_dist_assets(&dir).unwrap_or_else(|e| panic!("{e}"));

        insert_generated_css(
            &mut assets,
            vec![("feed-item.css".to_owned(), ".x{}".to_owned())],
        );

        assert!(assets.contains_key("index.js"));
        assert!(assets.contains_key("feed-item.css"));
        assert_eq!(assets.len(), 2);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn content_hashed_chunks_detected() {
        assert!(is_content_hashed("chunk-NKNSLYVV.js"));
        assert!(is_content_hashed("chunk-3QJD3BDH.js.map"));
        assert!(!is_content_hashed("index.js"));
        assert!(!is_content_hashed("index.js.map"));
    }

    #[test]
    fn content_hashed_chunks_are_immutable() {
        let dir = temp_dir("immutable");
        fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        fs::write(dir.join("chunk-NKNSLYVV.js"), "export const x = 1;")
            .unwrap_or_else(|e| panic!("{e}"));
        let assets = load_dist_assets(&dir).unwrap_or_else(|e| panic!("{e}"));

        let response = asset_response(&assets, "chunk-NKNSLYVV.js")
            .unwrap_or_else(|| panic!("expected hashed chunk"));
        assert_eq!(
            response
                .headers()
                .get(actix_web::http::header::CACHE_CONTROL)
                .and_then(|value| value.to_str().ok()),
            Some("public, max-age=31536000, immutable")
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
