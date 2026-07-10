// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

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
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// A component discovered from an external source, ready for registration.
#[derive(Debug, Clone)]
pub struct DiscoveredComponent {
    /// The custom element tag name (e.g., "reactive-button")
    pub tag_name: String,
    /// The HTML template content
    pub html_content: String,
    /// The CSS content, if any
    pub css_content: Option<String>,
    /// Whether authored browser code owns this custom element tag.
    pub has_script: bool,
    /// The authored client script source for local components, when present.
    ///
    /// Carried verbatim (this crate does not depend on `webui-parser`) so the
    /// caller can scan it for `@observable`/`@attr` decorators to derive the
    /// component's hydration surface. `None` for npm components, which ship no
    /// scannable sibling source.
    pub script_content: Option<String>,
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
    if is_local_source(source) {
        ComponentSource::Path(PathBuf::from(source))
    } else {
        ComponentSource::NpmPackage(source.to_string())
    }
}

/// Returns `true` when a `--components` source string denotes a local
/// filesystem path rather than an npm package name or scope.
///
/// A source is a local path when it starts with `.`, `/`, `\`, or a Windows
/// drive letter (e.g. `C:\...`). Everything else — bare names like
/// `my-widget` and scopes like `@scope` / `@scope/pkg` — is an npm package.
///
/// This is the single source of truth for the classification; callers that
/// pre-resolve sources before handing them to [`discover_source`] (such as
/// `webui-press`, which must resolve local paths against its own working
/// directory while leaving npm names bare) should use it instead of
/// re-implementing the check.
#[must_use]
pub fn is_local_source(source: &str) -> bool {
    source.starts_with('.')
        || source.starts_with('/')
        || source.starts_with('\\')
        || (cfg!(windows) && source.len() >= 2 && source.as_bytes()[1] == b':')
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
                    let html_content = std::fs::read_to_string(path).with_context(|| {
                        format!("Failed to read component HTML: {}", path.display())
                    })?;
                    let css_path = path.with_extension("css");
                    let css_content = if css_path.exists() {
                        Some(std::fs::read_to_string(&css_path).with_context(|| {
                            format!("Failed to read component CSS: {}", css_path.display())
                        })?)
                    } else {
                        None
                    };
                    let script_content = read_sibling_script(path)?;
                    components.push(DiscoveredComponent {
                        tag_name: filename.to_string(),
                        html_content,
                        css_content,
                        has_script: script_content.is_some(),
                        script_content,
                        source: source.clone(),
                    });
                }
            }
        }
    }

    Ok(components)
}

/// Read a component's sibling client module source, preferring `.ts` over `.js`.
///
/// Returns `Ok(None)` only when neither sibling exists. A sibling that exists
/// but cannot be read (I/O error, or non-UTF-8 source, which
/// [`std::fs::read_to_string`] rejects) is a hard error rather than a silent
/// `None`: swallowing it would downgrade the component to a static host and drop
/// its entire hydration surface without warning. This mirrors the sibling HTML
/// and CSS reads above.
fn read_sibling_script(html_path: &Path) -> Result<Option<String>> {
    for ext in ["ts", "js"] {
        let candidate = html_path.with_extension(ext);
        if candidate.exists() {
            let source = std::fs::read_to_string(&candidate).with_context(|| {
                format!("Failed to read component script: {}", candidate.display())
            })?;
            return Ok(Some(source));
        }
    }
    Ok(None)
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
    fn test_is_local_source() {
        // Local paths
        assert!(is_local_source("./libs/shared"));
        assert!(is_local_source("../components"));
        assert!(is_local_source("/absolute/path"));
        assert!(is_local_source("\\unc\\path"));
        // npm packages / scopes
        assert!(!is_local_source("my-widget"));
        assert!(!is_local_source("@reactive-ui"));
        assert!(!is_local_source("@scope/button"));
    }

    #[test]
    fn test_discover_from_path_preserves_script_ownership() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("plain-card.html"), "<p>{{title}}</p>").unwrap();
        std::fs::write(
            tmp.path().join("interactive-card.html"),
            "<button>go</button>",
        )
        .unwrap();
        std::fs::write(tmp.path().join("interactive-card.ts"), "export {};").unwrap();

        let components = discover_from_path(tmp.path()).unwrap();
        let plain = components
            .iter()
            .find(|component| component.tag_name == "plain-card")
            .unwrap();
        let interactive = components
            .iter()
            .find(|component| component.tag_name == "interactive-card")
            .unwrap();

        assert!(!plain.has_script);
        assert!(interactive.has_script);
    }
}
