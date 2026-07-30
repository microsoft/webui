// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Load-once, in-memory cache of this example's client build `dist/` output.
//!
//! `esbuild --splitting` emits content-hashed shared chunks (`chunk-HASH.js`)
//! whose names are unknown until the client build runs, so the server cannot
//! hard-code an asset list or a fixed set of routes. Reading the tree once at
//! startup solves that and removes per-request filesystem work; it also means
//! an arbitrary request path can only ever resolve to a file that was present
//! at startup, so path traversal has no filesystem to traverse.

use anyhow::{Context, Result};
use bytes::Bytes;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// One file read from `dist/`, ready to serve without touching the filesystem.
#[derive(Clone)]
pub struct CachedAsset {
    /// MIME type guessed from the file extension.
    pub content_type: String,
    /// File contents. `Bytes` so cloning per response is a refcount bump.
    pub body: Bytes,
}

/// Read every file under `dist_dir` once, keyed by its path relative to
/// `dist_dir` with forward slashes (`"chunk-NKNSLYVV.js"`,
/// `"nested/asset.js"`).
///
/// Returns an empty map — not an error — when `dist_dir` does not exist, so a
/// caller can report one clear "run the client build first" message instead of
/// a directory-read failure.
///
/// # Errors
///
/// Returns an error if a directory or file under `dist_dir` exists but cannot
/// be read.
pub fn load_dist_assets(dist_dir: &Path) -> Result<HashMap<String, CachedAsset>> {
    let mut assets = HashMap::new();
    if !dist_dir.is_dir() {
        return Ok(assets);
    }

    // Iterative traversal (not recursive) so directory depth cannot grow the
    // call stack; `esbuild --outdir` output is flat in practice, but this stays
    // correct if that ever changes.
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

/// `true` when a filename carries an esbuild content hash, making it safe to
/// cache immutably.
///
/// Esbuild produces `chunk-{HASH}.js` for shared chunks and `{name}-{HASH}.js`
/// for hashed entry points, always an 8-character uppercase-alphanumeric
/// suffix. Bare entry points such as `index.js` are not hashed and must
/// revalidate, or a rebuild would be invisible to a returning visitor.
#[must_use]
pub fn is_content_hashed(relative: &str) -> bool {
    let name = relative.rsplit('/').next().unwrap_or(relative);
    if !name.ends_with(".js") && !name.ends_with(".js.map") {
        return false;
    }
    let stem = name
        .strip_suffix(".js.map")
        .or_else(|| name.strip_suffix(".js"))
        .unwrap_or("");
    stem.rsplit('-').next().is_some_and(|hash| {
        hash.len() == 8
            && hash
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    })
}

#[cfg(test)]
mod tests {
    use super::{is_content_hashed, load_dist_assets};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        std::env::temp_dir().join(format!(
            "webui-dist-assets-{label}-{}-{unique}",
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
        assert_eq!(assets.len(), 2);
        assert!(assets["index.js"].content_type.contains("javascript"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn nested_directories_are_keyed_with_forward_slashes() {
        let dir = temp_dir("nested");
        fs::create_dir_all(dir.join("sub")).unwrap_or_else(|e| panic!("{e}"));
        fs::write(dir.join("sub").join("asset.js"), "export const y = 2;")
            .unwrap_or_else(|e| panic!("{e}"));

        let assets = load_dist_assets(&dir).unwrap_or_else(|e| panic!("{e}"));
        assert!(assets.contains_key("sub/asset.js"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_dist_directory_yields_an_empty_map_not_an_error() {
        let assets = load_dist_assets(&temp_dir("missing")).unwrap_or_else(|e| panic!("{e}"));
        assert!(assets.is_empty());
    }

    #[test]
    fn content_hashed_chunks_detected() {
        assert!(is_content_hashed("chunk-NKNSLYVV.js"));
        assert!(is_content_hashed("chunk-3QJD3BDH.js.map"));
        assert!(is_content_hashed("mp-page-home-UFH4TZ7P.js"));
        assert!(is_content_hashed("app.v2-ABCDEFGH.js"));
        assert!(is_content_hashed("app.v2-ABCDEFGH.js.map"));
        assert!(!is_content_hashed("index.js"));
        assert!(!is_content_hashed("index.js.map"));
        assert!(!is_content_hashed("feed-item.css"));
    }
}
