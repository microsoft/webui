// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Cache of materialized package components.
//!
//! Caches discovered component data at `~/.webui/cache/components/` and
//! validates them against prepared inputs and the bytes actually loaded.

use anyhow::{Context, Result};
use expand_tilde::expand_tilde;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::DiscoveredComponent;
use crate::prepared::LoadedPackage;

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) struct CacheKey<'a> {
    pub(crate) namespace: &'a str,
    pub(crate) source: &'a str,
    pub(crate) package_json: &'a Path,
}

/// Serialized cache entry stored as JSON on disk.
#[derive(Serialize, Deserialize)]
struct CacheEntry {
    /// The original source identifier (e.g., `@scope/button`)
    source: String,
    // Fingerprint of prepared decisions and the bytes in this result.
    version_hash: u64,
    /// Discovered components from this source
    components: Vec<CachedComponent>,
}

/// A component stored in the cache.
#[derive(Serialize, Deserialize)]
struct CachedComponent {
    tag_name: String,
    html_content: String,
    css_content: Option<String>,
    is_client_owned: bool,
}

/// File-based component discovery cache.
///
/// Stores component data at `~/.webui/cache/components/`, validated against
/// prepared source inputs on every lookup.
pub struct DiscoveryCache {
    cache_dir: PathBuf,
}

impl DiscoveryCache {
    /// Open (or create) the cache directory.
    pub fn open() -> Result<Self> {
        let home = expand_tilde(&PathBuf::from("~"))
            .context("Could not determine home directory for component cache")?
            .into_owned();
        let cache_dir = home.join(".webui").join("cache").join("components");
        fs::create_dir_all(&cache_dir).with_context(|| {
            format!("Failed to create cache directory: {}", cache_dir.display())
        })?;
        Ok(Self { cache_dir })
    }

    /// Derive a cache filename from the source identifier and package path.
    fn cache_key(namespace: &str, source: &str, pkg_json_path: &Path) -> String {
        let mut hasher = DefaultHasher::new();
        namespace.hash(&mut hasher);
        source.hash(&mut hasher);
        pkg_json_path.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    /// Look up cached components for a source. Returns `None` if the cache
    /// is missing, corrupt, or invalidated.
    pub(crate) fn get(
        &self,
        lookup: &CacheKey<'_>,
        fingerprint: u64,
    ) -> Result<Option<Vec<DiscoveredComponent>>> {
        let key = Self::cache_key(lookup.namespace, lookup.source, lookup.package_json);
        let cache_file = self.cache_dir.join(format!("{key}.json"));

        if !cache_file.exists() {
            return Ok(None);
        }

        // Gracefully handle corrupt cache files
        let content = match fs::read_to_string(&cache_file) {
            Ok(c) => c,
            Err(_) => return Ok(None),
        };

        let entry: CacheEntry = match serde_json::from_str(&content) {
            Ok(e) => e,
            Err(_) => return Ok(None),
        };

        // Validate version hash
        if entry.version_hash != fingerprint {
            return Ok(None);
        }

        let components = entry
            .components
            .into_iter()
            .map(|c| DiscoveredComponent {
                tag_name: c.tag_name,
                html_content: c.html_content,
                css_content: c.css_content,
                is_client_owned: c.is_client_owned,
                source: entry.source.clone(),
            })
            .collect();

        Ok(Some(components))
    }

    /// Store discovered components in the cache using atomic write.
    pub(crate) fn put(&self, lookup: &CacheKey<'_>, loaded: &LoadedPackage) -> Result<()> {
        let key = Self::cache_key(lookup.namespace, lookup.source, lookup.package_json);
        let cache_file = self.cache_dir.join(format!("{key}.json"));

        let entry = CacheEntry {
            source: lookup.source.to_string(),
            version_hash: loaded.fingerprint(),
            components: loaded
                .components()
                .iter()
                .map(|c| CachedComponent {
                    tag_name: c.tag_name.clone(),
                    html_content: c.html_content.clone(),
                    css_content: c.css_content.clone(),
                    is_client_owned: c.is_client_owned,
                })
                .collect(),
        };

        let json = serde_json::to_string(&entry).context("Failed to serialize cache entry")?;

        // Write to temp file then rename for atomic operation
        // (prevents corruption from concurrent builds)
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp_file =
            self.cache_dir
                .join(format!("{key}.{}.{}.tmp", std::process::id(), sequence));
        fs::write(&temp_file, &json)
            .with_context(|| format!("Failed to write temp cache file: {}", temp_file.display()))?;
        fs::rename(&temp_file, &cache_file)
            .with_context(|| format!("Failed to finalize cache file: {}", cache_file.display()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{prepared, ComponentFile, ComponentFileSource};

    fn lookup<'a>(source: &'a str, package_json: &'a Path) -> CacheKey<'a> {
        CacheKey {
            namespace: "test",
            source,
            package_json,
        }
    }

    fn input(root: &Path, name: &str) -> ComponentFile {
        ComponentFile {
            package_root: root.into(),
            path: root.join(name),
        }
    }

    fn fixture() -> Result<(tempfile::TempDir, PathBuf, Vec<ComponentFileSource>)> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("node_modules/test-pkg");
        fs::create_dir_all(&root)?;
        let root = root.canonicalize()?;
        let package_json = root.join("package.json");
        fs::write(
            &package_json,
            r#"{"version":1,"webui":{"components":["./"]}}"#,
        )?;
        fs::write(root.join("test-comp.html"), "<div>test</div>")?;
        let inputs = vec![ComponentFileSource {
            tag_name: "test-comp".to_string(),
            html: input(&root, "test-comp.html"),
            css: None,
            is_client_owned: false,
        }];
        Ok((temp, package_json, inputs))
    }

    #[test]
    fn test_cache_round_trip() -> Result<()> {
        let cache = DiscoveryCache::open()?;
        let (_temp, package_json, mut inputs) = fixture()?;
        let root = inputs[0].html.package_root.clone();
        fs::write(root.join("test-comp.css"), ".test { color: red; }")?;
        inputs[0].css = Some(input(&root, "test-comp.css"));
        let loaded = prepared::load("test-pkg", &fs::read_to_string(&package_json)?, inputs)?;
        let key = lookup("test-pkg", &package_json);
        cache.put(&key, &loaded)?;
        let cached = cache.get(&key, loaded.fingerprint())?.unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].tag_name, "test-comp");
        assert_eq!(cached[0].html_content, "<div>test</div>");
        assert!(!cached[0].is_client_owned);
        assert_eq!(
            cached[0].css_content.as_deref(),
            Some(".test { color: red; }")
        );
        Ok(())
    }

    #[test]
    fn test_cache_invalidation_on_content_change() -> Result<()> {
        let cache = DiscoveryCache::open()?;
        let (_temp, package_json, inputs) = fixture()?;
        let before = prepared::fingerprint(r#"{"version":1}"#, &inputs)?;
        let after = prepared::fingerprint(r#"{"version":2}"#, &inputs)?;
        let loaded = prepared::load("test-pkg", r#"{"version":1}"#, inputs)?;
        let key = lookup("test-pkg", &package_json);
        cache.put(&key, &loaded)?;
        assert!(cache.get(&key, before)?.is_some());
        assert!(cache.get(&key, after)?.is_none());
        Ok(())
    }

    #[test]
    fn test_cache_miss_for_unknown_source() {
        let cache = DiscoveryCache::open().unwrap();
        let tmp = tempfile::TempDir::new().unwrap();

        let pkg_json = tmp.path().join("package.json");
        fs::write(&pkg_json, r#"{"name":"unknown"}"#).unwrap();

        let cached = cache.get(&lookup("unknown-pkg", &pkg_json), 0).unwrap();
        assert!(cached.is_none());
    }

    #[test]
    fn test_cache_handles_corrupt_file() {
        let cache = DiscoveryCache::open().unwrap();
        let tmp = tempfile::TempDir::new().unwrap();

        let pkg_json = tmp.path().join("package.json");
        fs::write(&pkg_json, r#"{"name":"test"}"#).unwrap();

        // Write corrupt data to the cache location
        let key = DiscoveryCache::cache_key("test", "test-pkg", &pkg_json);
        let cache_file = cache.cache_dir.join(format!("{key}.json"));
        fs::write(&cache_file, "NOT VALID JSON!!!").unwrap();

        // Should gracefully return None, not error
        let cached = cache.get(&lookup("test-pkg", &pkg_json), 0).unwrap();
        assert!(cached.is_none());
    }

    #[test]
    fn test_cache_invalidation_tracks_optional_styles_and_ownership() -> Result<()> {
        let (_temp, _package_json, mut inputs) = fixture()?;
        let before = prepared::fingerprint("{}", &inputs)?;
        let root = inputs[0].html.package_root.clone();
        fs::write(root.join("test-comp.css"), "")?;
        inputs[0].css = Some(input(&root, "test-comp.css"));
        let styled = prepared::fingerprint("{}", &inputs)?;
        assert_ne!(styled, before);
        inputs[0].is_client_owned = true;
        assert_ne!(prepared::fingerprint("{}", &inputs)?, styled);
        Ok(())
    }

    #[test]
    fn test_fingerprint_tracks_prepared_inventory() -> Result<()> {
        let (_temp, _package_json, inputs) = fixture()?;
        assert_ne!(
            prepared::fingerprint("{}", &[])?,
            prepared::fingerprint("{}", &inputs)?
        );
        Ok(())
    }

    #[test]
    fn content_aba_is_published_under_observed_bytes_not_the_lookup_fingerprint() -> Result<()> {
        use crate::{DiscoveryPlugin, WebUIDiscoveryPlugin};
        let (temp, package_json, inputs) = fixture()?;
        let template = inputs[0].html.path.clone();
        let modified = fs::metadata(&template)?.modified()?;
        let metadata = fs::read_to_string(&package_json)?;
        let original = prepared::fingerprint(&metadata, &inputs)?;
        fs::write(&template, "<div>transient</div>")?;
        let loaded = prepared::load("test-pkg", &metadata, inputs)?;
        fs::write(&template, "<div>test</div>")?;
        fs::OpenOptions::new()
            .write(true)
            .open(&template)?
            .set_modified(modified)?;
        assert_ne!(loaded.fingerprint(), original);
        let cache = DiscoveryCache::open()?;
        let key = CacheKey {
            namespace: WebUIDiscoveryPlugin.cache_namespace(),
            source: "test-pkg",
            package_json: &package_json,
        };
        cache.put(&key, &loaded)?;
        assert!(cache.get(&key, original)?.is_none());
        let result = crate::discover_source("test-pkg", temp.path())?;
        assert_eq!(result.components[0].html_content, "<div>test</div>");
        Ok(())
    }
}
