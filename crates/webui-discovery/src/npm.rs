// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! npm package resolution for external component discovery.
//!
//! Resolves npm packages from `node_modules/` using Node.js-style upward
//! traversal. Reads `package.json` exports for template and styles, and
//! parses the Custom Elements Manifest for component tag names.

use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Component, Path, PathBuf};

use super::cache::DiscoveryCache;
use super::DiscoveredComponent;

/// Maximum file size for package.json and custom elements manifests (10 MB).
const MAX_MANIFEST_SIZE: u64 = 10 * 1024 * 1024;

/// Conditional export keys in priority order for fallback resolution.
const EXPORT_PRIORITY: &[&str] = &["default", "import", "require"];

/// Find `node_modules/` by walking up from `start` directory.
fn find_node_modules(start: &Path) -> Result<PathBuf> {
    let mut current = Some(start);
    while let Some(dir) = current {
        let candidate = dir.join("node_modules");
        if candidate.is_dir() {
            return Ok(candidate);
        }
        current = dir.parent();
    }
    bail!(
        "Could not find node_modules/ directory \
         (searched upward from {})",
        start.display()
    );
}

/// Check if a package name is a bare scope (e.g., `@reactive-ui` without a sub-package).
fn is_bare_scope(name: &str) -> bool {
    name.starts_with('@') && !name.contains('/')
}

/// Validate that a relative path from package.json does not escape the package directory.
///
/// Rejects absolute paths, root-relative paths, and any `..` components
/// to prevent path traversal attacks.
fn validate_relative_path(rel_path: &str, field_name: &str) -> Result<()> {
    let path = Path::new(rel_path);
    if path.is_absolute() || rel_path.starts_with('/') || rel_path.starts_with('\\') {
        bail!("Absolute path not allowed in {}: {}", field_name, rel_path);
    }
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        bail!(
            "Parent directory traversal (..) not allowed in {}: {}",
            field_name,
            rel_path
        );
    }
    Ok(())
}

/// Read a file with a size limit to prevent denial-of-service via oversized manifests.
fn read_to_string_limited(path: &Path, max_size: u64) -> Result<String> {
    let metadata =
        fs::metadata(path).with_context(|| format!("Failed to stat {}", path.display()))?;
    if metadata.len() > max_size {
        bail!(
            "File too large ({} bytes, max {}): {}",
            metadata.len(),
            max_size,
            path.display()
        );
    }
    fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))
}

/// Resolve an npm package or scope to discovered components.
pub fn resolve(
    name: &str,
    cwd: &Path,
    cache: &mut DiscoveryCache,
) -> Result<Vec<DiscoveredComponent>> {
    let node_modules = find_node_modules(cwd)?;

    if is_bare_scope(name) {
        resolve_scoped(name, &node_modules, cache)
    } else {
        resolve_single(name, &node_modules, cache)
    }
}

/// Enumerate all sub-packages under a scoped directory (e.g., `@reactive-ui/*`).
fn resolve_scoped(
    scope: &str,
    node_modules: &Path,
    cache: &mut DiscoveryCache,
) -> Result<Vec<DiscoveredComponent>> {
    let scope_dir = node_modules.join(scope);
    if !scope_dir.is_dir() {
        bail!(
            "Scoped package directory not found: {}",
            scope_dir.display()
        );
    }

    let mut all = Vec::new();
    for entry in fs::read_dir(&scope_dir)
        .with_context(|| format!("Failed to read scope directory: {}", scope_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let sub_name = format!("{}/{}", scope, entry.file_name().to_string_lossy());
        // Sub-packages without WebUI exports are expected — skip silently.
        if let Ok(components) = resolve_single(&sub_name, node_modules, cache) {
            all.extend(components);
        }
    }

    Ok(all)
}

/// Resolve a single npm package to discovered components.
fn resolve_single(
    name: &str,
    node_modules: &Path,
    cache: &mut DiscoveryCache,
) -> Result<Vec<DiscoveredComponent>> {
    let pkg_dir = node_modules.join(name);

    // Resolve symlinks (pnpm, npm workspaces, yarn)
    let pkg_dir = fs::canonicalize(&pkg_dir).with_context(|| {
        format!(
            "Package not found or broken symlink: {} (looked in {})",
            name,
            node_modules.display()
        )
    })?;

    // Verify resolved path is still within node_modules (prevent symlink escape)
    let node_modules_canon = fs::canonicalize(node_modules).with_context(|| {
        format!(
            "Failed to resolve node_modules path: {}",
            node_modules.display()
        )
    })?;
    if !pkg_dir.starts_with(&node_modules_canon) {
        bail!(
            "Package symlink escapes node_modules directory: {} resolves to {}",
            name,
            pkg_dir.display()
        );
    }

    if !pkg_dir.is_dir() {
        bail!("Package path is not a directory: {}", pkg_dir.display());
    }

    let pkg_json_path = pkg_dir.join("package.json");
    if !pkg_json_path.exists() {
        bail!("No package.json found at {}", pkg_json_path.display());
    }

    // Check cache first
    if let Some(cached) = cache.get(name, &pkg_json_path)? {
        return Ok(cached);
    }

    // Read and parse package.json
    let pkg_json_content = read_to_string_limited(&pkg_json_path, MAX_MANIFEST_SIZE)?;
    let pkg_json: serde_json::Value = serde_json::from_str(&pkg_json_content)
        .with_context(|| format!("Failed to parse {}", pkg_json_path.display()))?;

    // Resolve exports
    let exports = pkg_json
        .get("exports")
        .with_context(|| format!("No 'exports' field in {}", pkg_json_path.display()))?;

    let template_rel = resolve_export(exports, "./template-webui.html").with_context(|| {
        format!(
            "No './template-webui.html' export in {}",
            pkg_json_path.display()
        )
    })?;
    validate_relative_path(&template_rel, "exports[\"./template-webui.html\"]")?;
    let template_path = pkg_dir.join(&template_rel);

    let styles_rel = resolve_export(exports, "./styles.css");
    let styles_path = if let Some(ref rel) = styles_rel {
        validate_relative_path(rel, "exports[\"./styles.css\"]")?;
        Some(pkg_dir.join(rel))
    } else {
        None
    };

    // Get custom elements manifest
    let cem_rel = pkg_json
        .get("customElements")
        .and_then(|v| v.as_str())
        .with_context(|| format!("No 'customElements' field in {}", pkg_json_path.display()))?;
    validate_relative_path(cem_rel, "customElements")?;
    let cem_path = pkg_dir.join(cem_rel);

    // Parse custom elements manifest for tag names
    let tag_names = parse_custom_elements_manifest(&cem_path)?;
    if tag_names.is_empty() {
        bail!(
            "No component tag names found in custom elements manifest: {}",
            cem_path.display()
        );
    }

    // Read template HTML
    let html_content = read_to_string_limited(&template_path, MAX_MANIFEST_SIZE)
        .with_context(|| format!("Failed to read template: {}", template_path.display()))?;

    // Read CSS content (optional)
    let css_content = match &styles_path {
        Some(css_path) if css_path.exists() => Some(
            read_to_string_limited(css_path, MAX_MANIFEST_SIZE)
                .with_context(|| format!("Failed to read styles: {}", css_path.display()))?,
        ),
        _ => None,
    };

    // Create one DiscoveredComponent per tag name
    let components: Vec<DiscoveredComponent> = tag_names
        .into_iter()
        .map(|tag_name| DiscoveredComponent {
            tag_name,
            html_content: html_content.clone(),
            css_content: css_content.clone(),
            source: name.to_string(),
        })
        .collect();

    // Update cache
    cache.put(name, &pkg_json_path, &components)?;

    Ok(components)
}

/// Resolve an export path from the `exports` field in `package.json`.
///
/// Handles two common formats:
/// - Direct string: `"./template-webui.html": "./dist/template.html"`
/// - Conditional object: `"./template-webui.html": { "default": "./dist/template.html" }`
fn resolve_export(exports: &serde_json::Value, key: &str) -> Option<String> {
    match exports.get(key)? {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(obj) => {
            // Use deterministic priority order for conditional exports
            for key in EXPORT_PRIORITY {
                if let Some(serde_json::Value::String(s)) = obj.get(*key) {
                    return Some(s.clone());
                }
            }
            None
        }
        _ => None,
    }
}

/// Parse a Custom Elements Manifest JSON file to extract component tag names.
///
/// Follows the Custom Elements Manifest spec:
/// `modules[].declarations[].tagName`
fn parse_custom_elements_manifest(path: &Path) -> Result<Vec<String>> {
    let content = read_to_string_limited(path, MAX_MANIFEST_SIZE)
        .with_context(|| format!("Custom elements manifest: {}", path.display()))?;
    let manifest: serde_json::Value = serde_json::from_str(&content).with_context(|| {
        format!(
            "Failed to parse custom elements manifest: {}",
            path.display()
        )
    })?;

    let mut seen = std::collections::HashSet::new();
    let mut tag_names = Vec::new();

    if let Some(modules) = manifest.get("modules").and_then(|v| v.as_array()) {
        for module in modules {
            if let Some(declarations) = module.get("declarations").and_then(|v| v.as_array()) {
                for decl in declarations {
                    if let Some(tag_name) = decl.get("tagName").and_then(|v| v.as_str()) {
                        if seen.insert(tag_name) {
                            tag_names.push(tag_name.to_string());
                        }
                    }
                }
            }
        }
    }

    Ok(tag_names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_npm_package(dir: &Path, name: &str, tag_name: &str, html: &str, css: Option<&str>) {
        let pkg_dir = dir.join(name);
        fs::create_dir_all(&pkg_dir).unwrap();

        // Create template
        fs::write(pkg_dir.join("template-webui.html"), html).unwrap();

        // Create styles (optional)
        if let Some(css_content) = css {
            fs::write(pkg_dir.join("styles.css"), css_content).unwrap();
        }

        // Create custom elements manifest
        let manifest = serde_json::json!({
            "schemaVersion": "1.0.0",
            "modules": [{
                "kind": "javascript-module",
                "path": "src/index.js",
                "declarations": [{
                    "kind": "class",
                    "name": "MyComponent",
                    "tagName": tag_name
                }]
            }]
        });
        fs::write(
            pkg_dir.join("custom-elements.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        // Create package.json with exports
        let mut exports = serde_json::json!({
            "./template-webui.html": "./template-webui.html",
        });
        if css.is_some() {
            exports["./styles.css"] = serde_json::json!("./styles.css");
        }
        let pkg_json = serde_json::json!({
            "name": name,
            "version": "1.0.0",
            "customElements": "./custom-elements.json",
            "exports": exports
        });
        fs::write(
            pkg_dir.join("package.json"),
            serde_json::to_string_pretty(&pkg_json).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn test_find_node_modules_in_cwd() {
        let tmp = TempDir::new().unwrap();
        let nm = tmp.path().join("node_modules");
        fs::create_dir(&nm).unwrap();

        let result = find_node_modules(tmp.path());
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap().canonicalize().unwrap(),
            nm.canonicalize().unwrap()
        );
    }

    #[test]
    fn test_find_node_modules_walks_up() {
        let tmp = TempDir::new().unwrap();
        let nm = tmp.path().join("node_modules");
        fs::create_dir(&nm).unwrap();
        let sub = tmp.path().join("packages").join("my-app");
        fs::create_dir_all(&sub).unwrap();

        let result = find_node_modules(&sub);
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap().canonicalize().unwrap(),
            nm.canonicalize().unwrap()
        );
    }

    #[test]
    fn test_find_node_modules_not_found() {
        let tmp = TempDir::new().unwrap();
        let result = find_node_modules(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_is_bare_scope() {
        assert!(is_bare_scope("@reactive-ui"));
        assert!(is_bare_scope("@scope"));
        assert!(!is_bare_scope("@scope/button"));
        assert!(!is_bare_scope("my-widget"));
    }

    #[test]
    fn test_resolve_single_package() {
        let tmp = TempDir::new().unwrap();
        let nm = tmp.path().join("node_modules");
        fs::create_dir(&nm).unwrap();

        create_npm_package(
            &nm,
            "my-widget",
            "my-widget",
            "<div><slot></slot></div>",
            Some(".widget { color: blue; }"),
        );

        let mut cache = DiscoveryCache::open().unwrap();
        let result = resolve("my-widget", tmp.path(), &mut cache);
        assert!(result.is_ok());

        let components = result.unwrap();
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].tag_name, "my-widget");
        assert_eq!(components[0].html_content, "<div><slot></slot></div>");
        assert_eq!(
            components[0].css_content.as_deref(),
            Some(".widget { color: blue; }")
        );
    }

    #[test]
    fn test_resolve_scoped_package() {
        let tmp = TempDir::new().unwrap();
        let nm = tmp.path().join("node_modules");
        let scope_dir = nm.join("@mylib");
        fs::create_dir_all(&scope_dir).unwrap();

        create_npm_package(
            &scope_dir,
            "button",
            "mylib-button",
            "<button><slot></slot></button>",
            Some(".btn { padding: 8px; }"),
        );
        create_npm_package(
            &scope_dir,
            "text",
            "mylib-text",
            "<span><slot></slot></span>",
            None,
        );

        let mut cache = DiscoveryCache::open().unwrap();
        let result = resolve("@mylib", tmp.path(), &mut cache);
        assert!(result.is_ok());

        let components = result.unwrap();
        assert_eq!(components.len(), 2);

        let names: Vec<&str> = components.iter().map(|c| c.tag_name.as_str()).collect();
        assert!(names.contains(&"mylib-button"));
        assert!(names.contains(&"mylib-text"));
    }

    #[test]
    fn test_resolve_missing_package() {
        let tmp = TempDir::new().unwrap();
        let nm = tmp.path().join("node_modules");
        fs::create_dir(&nm).unwrap();

        let mut cache = DiscoveryCache::open().unwrap();
        let result = resolve("nonexistent", tmp.path(), &mut cache);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_export_direct_string() {
        let exports = serde_json::json!({
            "./template-webui.html": "./dist/template.html"
        });
        let result = resolve_export(&exports, "./template-webui.html");
        assert_eq!(result, Some("./dist/template.html".to_string()));
    }

    #[test]
    fn test_resolve_export_conditional_default() {
        let exports = serde_json::json!({
            "./template-webui.html": {
                "import": "./dist/template.mjs",
                "default": "./dist/template.html"
            }
        });
        let result = resolve_export(&exports, "./template-webui.html");
        assert_eq!(result, Some("./dist/template.html".to_string()));
    }

    #[test]
    fn test_resolve_export_conditional_fallback() {
        let exports = serde_json::json!({
            "./template-webui.html": {
                "import": "./dist/template.mjs"
            }
        });
        let result = resolve_export(&exports, "./template-webui.html");
        assert_eq!(result, Some("./dist/template.mjs".to_string()));
    }

    #[test]
    fn test_resolve_export_missing() {
        let exports = serde_json::json!({
            "./other.html": "./dist/other.html"
        });
        let result = resolve_export(&exports, "./template-webui.html");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_custom_elements_manifest() {
        let tmp = TempDir::new().unwrap();
        let manifest = serde_json::json!({
            "schemaVersion": "1.0.0",
            "modules": [
                {
                    "kind": "javascript-module",
                    "path": "src/button.js",
                    "declarations": [
                        {
                            "kind": "class",
                            "name": "MyButton",
                            "tagName": "my-button"
                        }
                    ]
                },
                {
                    "kind": "javascript-module",
                    "path": "src/text.js",
                    "declarations": [
                        {
                            "kind": "class",
                            "name": "MyText",
                            "tagName": "my-text"
                        },
                        {
                            "kind": "variable",
                            "name": "VERSION"
                        }
                    ]
                }
            ]
        });

        let cem_path = tmp.path().join("custom-elements.json");
        fs::write(&cem_path, serde_json::to_string_pretty(&manifest).unwrap()).unwrap();

        let result = parse_custom_elements_manifest(&cem_path);
        assert!(result.is_ok());

        let tag_names = result.unwrap();
        assert_eq!(tag_names, vec!["my-button", "my-text"]);
    }

    #[test]
    fn test_parse_custom_elements_manifest_empty() {
        let tmp = TempDir::new().unwrap();
        let manifest = serde_json::json!({
            "schemaVersion": "1.0.0",
            "modules": []
        });

        let cem_path = tmp.path().join("custom-elements.json");
        fs::write(&cem_path, serde_json::to_string_pretty(&manifest).unwrap()).unwrap();

        let result = parse_custom_elements_manifest(&cem_path);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_parse_custom_elements_manifest_deduplicates() {
        let tmp = TempDir::new().unwrap();
        let manifest = serde_json::json!({
            "schemaVersion": "1.0.0",
            "modules": [
                {
                    "kind": "javascript-module",
                    "declarations": [{ "kind": "class", "tagName": "my-button" }]
                },
                {
                    "kind": "javascript-module",
                    "declarations": [{ "kind": "class", "tagName": "my-button" }]
                }
            ]
        });

        let cem_path = tmp.path().join("custom-elements.json");
        fs::write(&cem_path, serde_json::to_string(&manifest).unwrap()).unwrap();

        let tag_names = parse_custom_elements_manifest(&cem_path).unwrap();
        assert_eq!(tag_names, vec!["my-button"]);
    }

    #[test]
    fn test_cached_result_is_reused() {
        let tmp = TempDir::new().unwrap();
        let nm = tmp.path().join("node_modules");
        fs::create_dir(&nm).unwrap();

        create_npm_package(&nm, "cached-pkg", "cached-comp", "<div>cached</div>", None);

        let mut cache = DiscoveryCache::open().unwrap();

        // First resolve: populates cache
        let first = resolve("cached-pkg", tmp.path(), &mut cache).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].tag_name, "cached-comp");

        // Second resolve: should hit cache
        let second = resolve("cached-pkg", tmp.path(), &mut cache).unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].tag_name, "cached-comp");
    }

    #[test]
    fn test_validate_relative_path_rejects_absolute() {
        let result = validate_relative_path("/etc/passwd", "exports");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Absolute path"));
    }

    #[test]
    fn test_validate_relative_path_rejects_parent_traversal() {
        let result = validate_relative_path("../../etc/passwd", "exports");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains(".."));
    }

    #[test]
    fn test_validate_relative_path_rejects_hidden_traversal() {
        let result = validate_relative_path("foo/../../../etc/passwd", "exports");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_relative_path_accepts_valid() {
        assert!(validate_relative_path("./dist/template.html", "exports").is_ok());
        assert!(validate_relative_path("template-webui.html", "exports").is_ok());
        assert!(validate_relative_path("dist/nested/file.css", "exports").is_ok());
    }

    #[test]
    fn test_resolve_rejects_path_traversal_in_exports() {
        let tmp = TempDir::new().unwrap();
        let nm = tmp.path().join("node_modules");
        let pkg_dir = nm.join("evil-pkg");
        fs::create_dir_all(&pkg_dir).unwrap();

        // Malicious package.json with path traversal in exports
        let pkg_json = serde_json::json!({
            "name": "evil-pkg",
            "version": "1.0.0",
            "customElements": "./custom-elements.json",
            "exports": {
                "./template-webui.html": "../../../etc/passwd"
            }
        });
        fs::write(
            pkg_dir.join("package.json"),
            serde_json::to_string(&pkg_json).unwrap(),
        )
        .unwrap();

        let mut cache = DiscoveryCache::open().unwrap();
        let result = resolve("evil-pkg", tmp.path(), &mut cache);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains(".."));
    }
}
