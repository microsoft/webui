//! Component discovery cache for avoiding repeated filesystem traversal.
//!
//! Caches discovered component data at `~/.webui/cache/components/` and
//! invalidates when the source package's `package.json` changes.

use anyhow::{Context, Result};
use expand_tilde::expand_tilde;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use super::DiscoveredComponent;

/// Serialized cache entry stored as JSON on disk.
#[derive(Serialize, Deserialize)]
struct CacheEntry {
    /// The original source identifier (e.g., `@scope/button`)
    source: String,
    /// Hash of the package.json content for invalidation
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
}

/// File-based component discovery cache.
///
/// Stores discovered component data at `~/.webui/cache/components/`
/// to avoid re-traversing npm packages on every build.
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
    fn cache_key(source: &str, pkg_json_path: &Path) -> String {
        let mut hasher = DefaultHasher::new();
        source.hash(&mut hasher);
        pkg_json_path.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    /// Compute a version hash from the content of a file.
    fn version_hash(path: &Path) -> Result<u64> {
        let content = fs::read(path)
            .with_context(|| format!("Failed to read for hashing: {}", path.display()))?;
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        Ok(hasher.finish())
    }

    /// Look up cached components for a source. Returns `None` if the cache
    /// is missing, corrupt, or invalidated.
    pub fn get(
        &self,
        source: &str,
        pkg_json_path: &Path,
    ) -> Result<Option<Vec<DiscoveredComponent>>> {
        let key = Self::cache_key(source, pkg_json_path);
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
        let current_hash = Self::version_hash(pkg_json_path)?;
        if entry.version_hash != current_hash {
            return Ok(None);
        }

        let components = entry
            .components
            .into_iter()
            .map(|c| DiscoveredComponent {
                tag_name: c.tag_name,
                html_content: c.html_content,
                css_content: c.css_content,
                source: entry.source.clone(),
            })
            .collect();

        Ok(Some(components))
    }

    /// Store discovered components in the cache using atomic write.
    pub fn put(
        &self,
        source: &str,
        pkg_json_path: &Path,
        components: &[DiscoveredComponent],
    ) -> Result<()> {
        let key = Self::cache_key(source, pkg_json_path);
        let cache_file = self.cache_dir.join(format!("{key}.json"));
        let version_hash = Self::version_hash(pkg_json_path)?;

        let entry = CacheEntry {
            source: source.to_string(),
            version_hash,
            components: components
                .iter()
                .map(|c| CachedComponent {
                    tag_name: c.tag_name.clone(),
                    html_content: c.html_content.clone(),
                    css_content: c.css_content.clone(),
                })
                .collect(),
        };

        let json = serde_json::to_string(&entry).context("Failed to serialize cache entry")?;

        // Write to temp file then rename for atomic operation
        // (prevents corruption from concurrent builds)
        let temp_file = self.cache_dir.join(format!("{key}.tmp"));
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

    #[test]
    fn test_cache_round_trip() {
        let cache = DiscoveryCache::open().unwrap();
        let tmp = tempfile::TempDir::new().unwrap();

        // Create a fake package.json
        let pkg_json = tmp.path().join("package.json");
        fs::write(&pkg_json, r#"{"name":"test","version":"1.0.0"}"#).unwrap();

        let components = vec![DiscoveredComponent {
            tag_name: "test-comp".to_string(),
            html_content: "<div>test</div>".to_string(),
            css_content: Some(".test { color: red; }".to_string()),
            source: "test-pkg".to_string(),
        }];

        // Put
        cache.put("test-pkg", &pkg_json, &components).unwrap();

        // Get
        let cached = cache.get("test-pkg", &pkg_json).unwrap();
        assert!(cached.is_some());
        let cached = cached.unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].tag_name, "test-comp");
        assert_eq!(cached[0].html_content, "<div>test</div>");
        assert_eq!(
            cached[0].css_content.as_deref(),
            Some(".test { color: red; }")
        );
    }

    #[test]
    fn test_cache_invalidation_on_content_change() {
        let cache = DiscoveryCache::open().unwrap();
        let tmp = tempfile::TempDir::new().unwrap();

        let pkg_json = tmp.path().join("package.json");
        fs::write(&pkg_json, r#"{"name":"test","version":"1.0.0"}"#).unwrap();

        let components = vec![DiscoveredComponent {
            tag_name: "test-comp".to_string(),
            html_content: "<div>v1</div>".to_string(),
            css_content: None,
            source: "test-pkg".to_string(),
        }];

        cache.put("test-pkg", &pkg_json, &components).unwrap();

        // Modify package.json
        fs::write(&pkg_json, r#"{"name":"test","version":"2.0.0"}"#).unwrap();

        // Cache should be invalidated
        let cached = cache.get("test-pkg", &pkg_json).unwrap();
        assert!(cached.is_none());
    }

    #[test]
    fn test_cache_miss_for_unknown_source() {
        let cache = DiscoveryCache::open().unwrap();
        let tmp = tempfile::TempDir::new().unwrap();

        let pkg_json = tmp.path().join("package.json");
        fs::write(&pkg_json, r#"{"name":"unknown"}"#).unwrap();

        let cached = cache.get("unknown-pkg", &pkg_json).unwrap();
        assert!(cached.is_none());
    }

    #[test]
    fn test_cache_handles_corrupt_file() {
        let cache = DiscoveryCache::open().unwrap();
        let tmp = tempfile::TempDir::new().unwrap();

        let pkg_json = tmp.path().join("package.json");
        fs::write(&pkg_json, r#"{"name":"test"}"#).unwrap();

        // Write corrupt data to the cache location
        let key = DiscoveryCache::cache_key("test-pkg", &pkg_json);
        let cache_file = cache.cache_dir.join(format!("{key}.json"));
        fs::write(&cache_file, "NOT VALID JSON!!!").unwrap();

        // Should gracefully return None, not error
        let cached = cache.get("test-pkg", &pkg_json).unwrap();
        assert!(cached.is_none());
    }
}
