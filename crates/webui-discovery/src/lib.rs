//! External component discovery for the WebUI framework.
//!
//! Discovers components from npm packages and local filesystem paths.
//! Returns [`DiscoveredComponent`] structs that callers can register
//! into their component registry.
//!
//! This crate has no dependency on `webui-parser` — it is pure discovery
//! logic reusable by CLI, FFI, and other host integrations.

mod cache;
mod npm;

use anyhow::{Context, Result};
use console::style;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const MAX_COMPONENT_FILE_SIZE: u64 = 1_048_576;

/// A component discovered from an external source, ready for registration.
#[derive(Debug, Clone)]
pub struct DiscoveredComponent {
    /// The custom element tag name (e.g., "reactive-button")
    pub tag_name: String,
    /// The HTML template content
    pub html_content: String,
    /// The CSS content, if any
    pub css_content: Option<String>,
    /// The original source identifier (for display/diagnostics)
    pub source: String,
}

/// Result of discovering components from a single source.
#[derive(Debug)]
pub struct DiscoveryResult {
    /// The original source string
    pub source: String,
    /// The discovered components
    pub components: Vec<DiscoveredComponent>,
}

/// Classification of a `--components` source string.
enum ComponentSource {
    /// npm package (scoped like `@scope` or unscoped like `my-widget`)
    NpmPackage(String),
    /// Local filesystem path
    Path(PathBuf),
}

/// Classify a source string as either an npm package or a local path.
///
/// - Starts with `.`, `/`, `\`, or contains a drive letter (Windows) → path
/// - Everything else → npm package
fn classify_source(source: &str) -> ComponentSource {
    if source.starts_with('.')
        || source.starts_with('/')
        || source.starts_with('\\')
        || (cfg!(windows) && source.len() >= 2 && source.as_bytes()[1] == b':')
    {
        ComponentSource::Path(PathBuf::from(source))
    } else {
        ComponentSource::NpmPackage(source.to_string())
    }
}

/// Discover components from a single source and register them into a component registry.
///
/// Returns a [`DiscoveryResult`] with the discovered components.
pub fn discover_source(source: &str, search_dir: &Path) -> Result<DiscoveryResult> {
    let mut cache = cache::DiscoveryCache::open()?;

    let components = match classify_source(source) {
        ComponentSource::NpmPackage(ref name) => npm::resolve(name, search_dir, &mut cache)?,
        ComponentSource::Path(ref path) => {
            let resolved = if path.is_relative() {
                search_dir.join(path)
            } else {
                path.clone()
            };
            let resolved = resolved
                .canonicalize()
                .with_context(|| format!("Component path not found: {}", path.display()))?;
            discover_from_path(&resolved)?
        }
    };

    Ok(DiscoveryResult {
        source: source.to_string(),
        components,
    })
}

/// Discover components from a local directory path.
///
/// Scans recursively for HTML files with hyphenated names (Web Components
/// convention) and pairs them with matching CSS files.
fn discover_from_path(dir: &Path) -> Result<Vec<DiscoveredComponent>> {
    let source = dir.display().to_string();
    let mut components = Vec::new();

    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().is_some_and(|ext| ext == "html") {
            if let Some(filename) = path.file_stem().and_then(|s| s.to_str()) {
                if filename.contains('-') {
                    let html_content = match read_component_file(path, "component HTML")? {
                        Some(content) => content,
                        None => continue,
                    };
                    let css_path = path.with_extension("css");
                    let css_content = if css_path.exists() {
                        match read_component_file(&css_path, "component CSS")? {
                            Some(content) => Some(content),
                            None => continue,
                        }
                    } else {
                        None
                    };
                    components.push(DiscoveredComponent {
                        tag_name: filename.to_string(),
                        html_content,
                        css_content,
                        source: source.clone(),
                    });
                }
            }
        }
    }

    Ok(components)
}

fn read_component_file(path: &Path, label: &str) -> Result<Option<String>> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("Failed to stat {label}: {}", path.display()))?;
    if metadata.len() > MAX_COMPONENT_FILE_SIZE {
        eprintln!(
            "  {} Skipping oversized file: {} ({} bytes)",
            style("⚠").yellow(),
            path.display(),
            metadata.len()
        );
        return Ok(None);
    }

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {label}: {}", path.display()))?;
    Ok(Some(content))
}

/// Collect the resolved local paths from source strings for file watching.
pub fn collect_watch_paths(sources: &[String], search_dir: &Path) -> Vec<PathBuf> {
    sources
        .iter()
        .filter_map(|source| {
            if let ComponentSource::Path(ref path) = classify_source(source) {
                let resolved = if path.is_relative() {
                    search_dir.join(path)
                } else {
                    path.clone()
                };
                resolved.canonicalize().ok()
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_classify_relative_path() {
        assert!(matches!(
            classify_source("./libs/shared"),
            ComponentSource::Path(_)
        ));
        assert!(matches!(
            classify_source("../components"),
            ComponentSource::Path(_)
        ));
    }

    #[test]
    fn test_classify_absolute_path() {
        assert!(matches!(
            classify_source("/absolute/path"),
            ComponentSource::Path(_)
        ));
    }

    #[test]
    fn test_classify_npm_package() {
        assert!(matches!(
            classify_source("my-widget"),
            ComponentSource::NpmPackage(_)
        ));
    }

    #[test]
    fn test_classify_scoped_npm_package() {
        assert!(matches!(
            classify_source("@reactive-ui"),
            ComponentSource::NpmPackage(_)
        ));
        assert!(matches!(
            classify_source("@scope/button"),
            ComponentSource::NpmPackage(_)
        ));
    }

    #[cfg(windows)]
    #[test]
    fn test_classify_windows_drive_path() {
        assert!(matches!(
            classify_source("C:\\components"),
            ComponentSource::Path(_)
        ));
    }

    #[test]
    fn test_discover_from_path_skips_oversized_html() {
        let tmp = TempDir::new().unwrap();
        let html_path = tmp.path().join("big-component.html");
        fs::write(
            &html_path,
            vec![b'a'; (MAX_COMPONENT_FILE_SIZE as usize) + 1],
        )
        .unwrap();

        let components = discover_from_path(tmp.path()).unwrap();
        assert!(components.is_empty());
    }

    #[test]
    fn test_discover_from_path_skips_component_with_oversized_css() {
        let tmp = TempDir::new().unwrap();
        let html_path = tmp.path().join("styled-component.html");
        let css_path = tmp.path().join("styled-component.css");
        fs::write(&html_path, "<div>ok</div>").unwrap();
        fs::write(
            &css_path,
            vec![b'b'; (MAX_COMPONENT_FILE_SIZE as usize) + 1],
        )
        .unwrap();

        let components = discover_from_path(tmp.path()).unwrap();
        assert!(components.is_empty());
    }
}
