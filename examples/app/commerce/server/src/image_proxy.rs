// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use actix_web::web::{self, Bytes};
use actix_web::HttpResponse;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Widths served by the commerce app templates.  Pre-generated AVIF
/// variants for each width live in `images/{width}/` subdirectories.
pub(crate) const SERVED_WIDTHS: &[u32] = &[64, 96, 384, 640, 1080];

/// Default width when the `w` query param is absent or empty (e.g.
/// during client-side hydration).
const DEFAULT_WIDTH: u32 = 640;

/// In-memory image cache that serves pre-generated AVIF product images.
///
/// At startup, every `images/{width}/{stem}.avif` file is loaded into
/// memory.  The proxy endpoint performs a direct `HashMap` lookup —
/// no runtime decoding, resizing, or encoding.
pub(crate) struct ImageCache {
    /// Maps `(stem, width)` → AVIF bytes.
    variants: HashMap<(String, u32), Bytes>,
    /// Number of unique image stems.
    stem_count: usize,
}

impl ImageCache {
    /// Load all pre-generated AVIF variants from `images/{width}/` dirs.
    pub(crate) fn load(images_dir: &Path) -> Result<Self> {
        let mut variants = HashMap::new();
        let mut stems = std::collections::HashSet::new();

        for &width in SERVED_WIDTHS {
            let dir = images_dir.join(width.to_string());
            if !dir.is_dir() {
                continue;
            }
            for entry in
                fs::read_dir(&dir).with_context(|| format!("Failed to read {}", dir.display()))?
            {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("avif") {
                    continue;
                }
                let stem = match path.file_stem().and_then(|s| s.to_str()) {
                    Some(s) if !s.is_empty() => s.to_string(),
                    _ => continue,
                };
                let bytes = fs::read(&path)
                    .with_context(|| format!("Failed to read {}", path.display()))?;
                stems.insert(stem.clone());
                variants.insert((stem, width), Bytes::from(bytes));
            }
        }

        Ok(Self {
            variants,
            stem_count: stems.len(),
        })
    }

    /// Number of unique image stems loaded.
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.stem_count
    }

    /// Total number of size variants loaded.
    #[must_use]
    pub(crate) fn variant_count(&self) -> usize {
        self.variants.len()
    }

    /// Look up a pre-generated variant.  Returns `None` if the stem or
    /// width is not found.  Callers should snap `width` to a value in
    /// [`SERVED_WIDTHS`] before calling.
    #[must_use]
    pub(crate) fn get(&self, stem: &str, width: u32) -> Option<Bytes> {
        self.variants.get(&(stem.to_string(), width)).cloned()
    }
}

/// Snap a requested width to the nearest served width that is ≥ the
/// request.  Falls back to the largest served width.
fn snap_width(requested: u32) -> u32 {
    SERVED_WIDTHS
        .iter()
        .copied()
        .find(|&w| w >= requested)
        .unwrap_or(*SERVED_WIDTHS.last().unwrap_or(&DEFAULT_WIDTH))
}

/// Deserialize an optional numeric value, treating empty strings as `None`.
fn empty_string_as_none<'de, D, T>(de: D) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    use serde::Deserialize;
    let opt: Option<String> = Option::deserialize(de)?;
    match opt.as_deref() {
        None | Some("") => Ok(None),
        Some(s) => s.parse::<T>().map(Some).map_err(serde::de::Error::custom),
    }
}

/// Query parameters for the image proxy endpoint.
#[derive(serde::Deserialize)]
pub(crate) struct ImageSizeQuery {
    #[serde(default, deserialize_with = "empty_string_as_none")]
    w: Option<u32>,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    #[allow(dead_code)]
    q: Option<u8>,
}

/// `GET /_image/{stem}?w=<width>&q=<quality>`
///
/// Serves pre-generated AVIF product images.  The `w` parameter is
/// snapped to the nearest served width.  When absent or empty, defaults
/// to [`DEFAULT_WIDTH`].
pub(crate) async fn serve_image(
    stem: web::Path<String>,
    query: web::Query<ImageSizeQuery>,
    data: web::Data<crate::app::AppState>,
) -> HttpResponse {
    let stem = stem.into_inner();
    let w = snap_width(query.w.unwrap_or(DEFAULT_WIDTH));

    match data.image_cache().get(&stem, w) {
        Some(bytes) => HttpResponse::Ok()
            .content_type("image/avif")
            .insert_header(("Cache-Control", "public, max-age=31536000, immutable"))
            .insert_header(("Vary", "Accept"))
            .body(bytes),
        None => HttpResponse::NotFound().body("Image not found"),
    }
}

#[cfg(test)]
mod tests {
    use super::{snap_width, ImageCache, SERVED_WIDTHS};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestImagesDir {
        path: PathBuf,
    }

    impl TestImagesDir {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos());
            let path = std::env::temp_dir().join(format!(
                "webui-commerce-image-cache-{}-{unique}",
                std::process::id()
            ));

            for &width in SERVED_WIDTHS {
                let dir = path.join(width.to_string());
                fs::create_dir_all(&dir).unwrap_or_else(|error| panic!("{error}"));
                write_fake_avif(&dir, "keyboard", width, 0x11);
                write_fake_avif(&dir, "baby-cap-white", width, 0x33);
            }

            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestImagesDir {
        fn drop(&mut self) {
            if self.path.exists() {
                fs::remove_dir_all(&self.path).unwrap_or_else(|error| panic!("{error}"));
            }
        }
    }

    fn write_fake_avif(dir: &Path, stem: &str, width: u32, seed: u8) {
        let payload_len = (width as usize / 4).max(1);
        let mut bytes = Vec::with_capacity(16 + payload_len);
        bytes.extend_from_slice(&[0, 0, 0, 32, b'f', b't', b'y', b'p']);
        bytes.extend_from_slice(b"avif");
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        for i in 0..payload_len {
            // Test fixture: byte values intentionally wrap mod 256.
            #[allow(clippy::cast_possible_truncation)]
            let byte = i as u8;
            bytes.push(seed.wrapping_add(byte));
        }
        fs::write(dir.join(format!("{stem}.avif")), bytes)
            .unwrap_or_else(|error| panic!("{error}"));
    }

    #[test]
    fn snap_rounds_up_to_nearest_served_width() {
        assert_eq!(snap_width(32), 64);
        assert_eq!(snap_width(64), 64);
        assert_eq!(snap_width(65), 96);
        assert_eq!(snap_width(400), 640);
        assert_eq!(snap_width(1080), 1080);
        assert_eq!(snap_width(2000), 1080);
    }

    #[test]
    fn cache_loads_all_variants() {
        let images = TestImagesDir::new();
        let cache = ImageCache::load(images.path()).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(cache.len(), 2, "expected both fake stems");
        assert_eq!(
            cache.variant_count(),
            cache.len() * SERVED_WIDTHS.len(),
            "every stem should have a variant for each served width"
        );
    }

    #[test]
    fn get_returns_bytes_for_valid_stem_and_width() {
        let images = TestImagesDir::new();
        let cache = ImageCache::load(images.path()).unwrap_or_else(|error| panic!("{error}"));
        let bytes = cache.get("keyboard", 640);
        assert!(bytes.is_some(), "expected keyboard at 640");
        let bytes = bytes.unwrap_or_else(|| panic!("missing keyboard at 640"));
        assert!(bytes.len() > 16, "expected AVIF header plus payload");
        assert_eq!(&bytes[4..8], b"ftyp");
    }

    #[test]
    fn smaller_width_produces_smaller_file() {
        let images = TestImagesDir::new();
        let cache = ImageCache::load(images.path()).unwrap_or_else(|error| panic!("{error}"));
        let small = cache
            .get("keyboard", 96)
            .unwrap_or_else(|| panic!("missing keyboard at 96"));
        let large = cache
            .get("keyboard", 640)
            .unwrap_or_else(|| panic!("missing keyboard at 640"));
        assert!(
            small.len() < large.len(),
            "96px should be smaller than 640px: {} vs {}",
            small.len(),
            large.len()
        );
    }

    #[test]
    fn get_returns_none_for_unknown_stem() {
        let images = TestImagesDir::new();
        let cache = ImageCache::load(images.path()).unwrap_or_else(|error| panic!("{error}"));
        assert!(cache.get("nonexistent", 640).is_none());
    }
}
