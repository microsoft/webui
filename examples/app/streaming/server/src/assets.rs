// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! HTTP response policy for this example's cached `dist/` assets.
//!
//! Loading and content-hash detection live in `webui-example-dist-assets`,
//! shared with the commerce example. What stays here is the part that is
//! genuinely this app's: merging the WebUI build's in-memory component CSS
//! into the same map, and choosing cache headers.

use actix_web::HttpResponse;
use bytes::Bytes;
use std::collections::HashMap;
use webui_example_dist_assets::{is_content_hashed, CachedAsset};

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

#[cfg(test)]
mod tests {
    use super::{asset_response, insert_generated_css};
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};
    use webui_example_dist_assets::load_dist_assets;

    fn temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        std::env::temp_dir().join(format!(
            "webui-streaming-assets-{label}-{}-{unique}",
            std::process::id()
        ))
    }

    fn header(
        response: &actix_web::HttpResponse,
        name: actix_web::http::header::HeaderName,
    ) -> String {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    }

    #[test]
    fn asset_response_serves_known_keys_and_rejects_everything_else() {
        let dir = temp_dir("response");
        fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        fs::write(dir.join("index.js"), "console.log('entry');").unwrap_or_else(|e| panic!("{e}"));
        let assets = load_dist_assets(&dir).unwrap_or_else(|e| panic!("{e}"));

        let response =
            asset_response(&assets, "index.js").unwrap_or_else(|| panic!("expected index.js"));
        assert!(header(&response, actix_web::http::header::CONTENT_TYPE).contains("javascript"));
        // The stable entry point is not content-hashed, so it must revalidate.
        assert_eq!(
            header(&response, actix_web::http::header::CACHE_CONTROL),
            "no-cache"
        );

        assert!(asset_response(&assets, "missing.js").is_none());
        // Path-traversal attempts never touch the filesystem — they simply
        // fail the exact-key lookup like any other unknown key.
        assert!(asset_response(&assets, "../../etc/passwd").is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn content_hashed_chunks_are_served_immutable() {
        let dir = temp_dir("immutable");
        fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        fs::write(dir.join("chunk-NKNSLYVV.js"), "export const x = 1;")
            .unwrap_or_else(|e| panic!("{e}"));
        let assets = load_dist_assets(&dir).unwrap_or_else(|e| panic!("{e}"));

        let response = asset_response(&assets, "chunk-NKNSLYVV.js")
            .unwrap_or_else(|| panic!("expected hashed chunk"));
        assert_eq!(
            header(&response, actix_web::http::header::CACHE_CONTROL),
            "public, max-age=31536000, immutable"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// The `link` and `module` CSS strategies emit
    /// `<link rel="stylesheet" href="feed-item.css">` into every shadow root
    /// plus a `<link rel="preload">` in `<head>`. Those files exist only in
    /// the build result, so without this merge every one of them 404s and the
    /// page renders unstyled.
    #[test]
    fn generated_component_css_is_served_as_revalidating_css() {
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
        let content_type = header(&response, actix_web::http::header::CONTENT_TYPE);
        assert!(content_type.starts_with("text/css"), "{content_type}");
        // Stable `<tag>.css` names are not content-hashed, so they must
        // revalidate rather than be cached immutably.
        assert_eq!(
            header(&response, actix_web::http::header::CACHE_CONTROL),
            "no-cache"
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
}
