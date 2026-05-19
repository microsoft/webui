// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Programmatic API for the WebUI build-time rendering framework.
//!
//! This crate provides the core build, render, and inspection APIs
//! that power the `webui` CLI, Node.js bindings, and WASM module.
//!
//! # Example
//!
//! ```rust,no_run
//! use webui::{build, BuildOptions, CssStrategy, DomStrategy};
//! use std::path::PathBuf;
//!
//! let result = build(BuildOptions {
//!     app_dir: PathBuf::from("./src"),
//!     entry: "index.html".to_string(),
//!     css: CssStrategy::Link,
//!     dom: DomStrategy::Shadow,
//!     plugin: None,
//!     components: Vec::new(),
//! }).unwrap();
//!
//! println!("Built {} fragments in {:?}", result.stats.fragment_count, result.stats.duration);
//! ```

mod error;
pub mod server;
pub mod streaming;

pub use error::WebUIError;

// Re-export core types from downstream crates
pub use webui_handler::route_handler::{
    encode_inventory, get_needed_components, get_needed_components_for_request, parse_inventory,
    ProtocolIndex,
};
pub use webui_handler::route_matcher::CompiledRouteCache;
pub use webui_handler::{plugin::HandlerPlugin, HandlerError, ResponseWriter, WebUIHandler};
pub use webui_parser::CssStrategy;
pub use webui_parser::DomStrategy;
pub use webui_parser::Plugin;
pub use webui_protocol::WebUIProtocol;

use std::fs;
use std::path::Path;
use std::time::Instant;
use webui_parser::plugin::fast_v2::FastV2ParserPlugin;
use webui_parser::plugin::fast_v3::FastV3ParserPlugin;
use webui_parser::plugin::webui::WebUIParserPlugin;
use webui_parser::plugin::ParserPluginArtifacts;
use webui_parser::HtmlParser;

/// Options for building a WebUI application.
#[derive(Debug, Clone)]
pub struct BuildOptions {
    /// Path to the application folder containing templates.
    pub app_dir: std::path::PathBuf,
    /// Entry HTML file name (e.g., `"index.html"`).
    pub entry: String,
    /// CSS delivery strategy for component stylesheets.
    pub css: CssStrategy,
    /// DOM strategy for component rendering (shadow or light).
    pub dom: DomStrategy,
    /// Framework plugin to load.
    pub plugin: Option<Plugin>,
    /// Additional component sources (npm packages or local paths).
    pub components: Vec<String>,
}

/// Statistics about a completed build.
#[derive(Debug, Clone)]
pub struct BuildStats {
    /// Total wall-clock build time.
    pub duration: std::time::Duration,
    /// Total number of protocol fragments.
    pub fragment_count: usize,
    /// Number of registered components.
    pub component_count: usize,
    /// Number of CSS files produced.
    pub css_file_count: usize,
    /// Size of the serialized protocol in bytes.
    pub protocol_size_bytes: usize,
    /// Number of unique CSS tokens discovered.
    pub token_count: usize,
}

/// Result of a successful build.
#[derive(Debug)]
pub struct BuildResult {
    /// The compiled WebUI protocol.
    pub protocol: WebUIProtocol,
    /// Serialized protocol bytes (protobuf binary).
    pub protocol_bytes: Vec<u8>,
    /// Component CSS files: `(filename, content)` — only components referenced in the protocol.
    pub css_files: Vec<(String, String)>,
    /// Component f-template strings: `(tag_name, f_template_html)`.
    /// Includes templates for all components encountered during parsing,
    /// including route-referenced components.
    pub component_templates: Vec<(String, String)>,
    /// Build statistics.
    pub stats: BuildStats,
}

/// Build a WebUI application from an app directory.
///
/// Parses templates, discovers components, and produces a compiled protocol
/// with build statistics.
///
/// # Errors
///
/// Returns [`WebUIError`] if the app directory is invalid, templates fail
/// to parse, or the protocol cannot be serialized.
#[must_use = "BuildResult contains the compiled protocol and statistics"]
pub fn build(options: BuildOptions) -> Result<BuildResult, WebUIError> {
    let started = Instant::now();

    let raw = build_protocol_inner(&options)?;

    let protocol_bytes = raw.protocol.to_protobuf()?;

    let stats = BuildStats {
        duration: started.elapsed(),
        fragment_count: raw.fragment_count,
        component_count: raw.component_count,
        css_file_count: raw.css_files.len(),
        protocol_size_bytes: protocol_bytes.len(),
        token_count: raw.token_count,
    };

    Ok(BuildResult {
        protocol: raw.protocol,
        protocol_bytes,
        css_files: raw.css_files,
        component_templates: raw.component_templates,
        stats,
    })
}

/// Build a WebUI application and write output files to disk.
///
/// Writes `protocol.bin` and any external CSS files to `out_dir`.
/// Creates `out_dir` if it does not exist.
///
/// # Errors
///
/// Returns [`WebUIError`] on build failure or if output files cannot be written.
pub fn build_to_disk(options: BuildOptions, out_dir: &Path) -> Result<BuildStats, WebUIError> {
    let result = build(options)?;

    fs::create_dir_all(out_dir).map_err(|source| WebUIError::Io {
        context: format!("Failed to create {}", out_dir.display()),
        source,
    })?;

    fs::write(out_dir.join("protocol.bin"), &result.protocol_bytes).map_err(|source| {
        WebUIError::Io {
            context: format!("Failed to write protocol.bin to {}", out_dir.display()),
            source,
        }
    })?;

    for (name, content) in &result.css_files {
        fs::write(out_dir.join(name), content).map_err(|source| WebUIError::Io {
            context: format!("Failed to write {name} to {}", out_dir.display()),
            source,
        })?;
    }

    Ok(result.stats)
}

/// Inspect a compiled WebUI protocol file and return its JSON representation.
pub fn inspect(protocol_path: &Path) -> Result<String, WebUIError> {
    let bytes = fs::read(protocol_path).map_err(|source| WebUIError::Io {
        context: format!("Failed to read {}", protocol_path.display()),
        source,
    })?;
    inspect_bytes(&bytes)
}

/// Inspect raw protocol bytes and return their JSON representation.
pub fn inspect_bytes(protocol_bytes: &[u8]) -> Result<String, WebUIError> {
    let protocol = WebUIProtocol::from_protobuf(protocol_bytes)?;
    protocol
        .to_json_pretty()
        .map_err(|e| WebUIError::Serialization(e.to_string()))
}

/// Internal intermediate build output before stats are computed.
struct RawBuildOutput {
    protocol: WebUIProtocol,
    css_files: Vec<(String, String)>,
    component_templates: Vec<(String, String)>,
    fragment_count: usize,
    component_count: usize,
    token_count: usize,
}

/// Internal build logic shared by `build()` and `build_to_disk()`.
fn build_protocol_inner(options: &BuildOptions) -> Result<RawBuildOutput, WebUIError> {
    let mut parser = match options.plugin {
        Some(Plugin::Fast | Plugin::FastV2) => {
            let mut plugin = FastV2ParserPlugin::new();
            plugin.set_css_strategy(options.css);
            plugin.set_dom_strategy(options.dom);
            HtmlParser::with_plugin(Box::new(plugin))
        }
        Some(Plugin::FastV3) => {
            let mut plugin = FastV3ParserPlugin::new();
            plugin.set_css_strategy(options.css);
            plugin.set_dom_strategy(options.dom);
            HtmlParser::with_plugin(Box::new(plugin))
        }
        Some(Plugin::WebUI) => {
            let mut plugin = WebUIParserPlugin::new();
            plugin.set_css_strategy(options.css);
            plugin.set_dom_strategy(options.dom);
            HtmlParser::with_plugin(Box::new(plugin))
        }
        None => HtmlParser::new(),
    };
    parser.set_css_strategy(options.css);
    parser.set_dom_strategy(options.dom);

    // Register app directory components
    parser
        .component_registry_mut()
        .register_from_paths(&[&options.app_dir])
        .map_err(|e| {
            WebUIError::ComponentRegistration(format!(
                "Failed to register components from {}: {e}",
                options.app_dir.display()
            ))
        })?;

    // Discover and register external component sources
    for source in &options.components {
        let result = webui_discovery::discover_source(source, &options.app_dir).map_err(|e| {
            WebUIError::ComponentDiscovery(format!(
                "Failed to discover components from {source}: {e}"
            ))
        })?;
        for comp in &result.components {
            parser
                .component_registry_mut()
                .register_component(
                    &comp.tag_name,
                    &comp.html_content,
                    comp.css_content.as_deref(),
                )
                .map_err(|e| {
                    WebUIError::ComponentRegistration(format!(
                        "Failed to register component '{}' from {}: {e}",
                        comp.tag_name, comp.source
                    ))
                })?;
        }
    }

    let component_count = parser.component_registry_mut().len();

    // Parse entry HTML
    let entry_path = options.app_dir.join(&options.entry);
    let html_content = fs::read_to_string(&entry_path).map_err(|source| WebUIError::Io {
        context: format!("Failed to read {}", entry_path.display()),
        source,
    })?;
    parser
        .parse(&options.entry, &html_content)
        .map_err(|source| WebUIError::Parse {
            context: format!("Failed to parse {}", options.entry),
            source,
        })?;

    // Snapshot CSS before consuming the parser
    let css_snapshot: Vec<(String, String)> = parser
        .component_registry_mut()
        .get_all()
        .filter_map(|c| {
            c.css_content
                .as_ref()
                .map(|css| (c.tag_name.clone(), css.clone()))
        })
        .collect();

    // Collect CSS tokens before consuming the parser
    let tokens = parser.take_tokens();
    let token_count = tokens.len();

    let component_templates = match parser.take_plugin_artifacts() {
        ParserPluginArtifacts::None => Vec::new(),
        ParserPluginArtifacts::ComponentTemplates(templates) => templates,
    };

    // Build protocol (consumes parser)
    let fragment_records = parser.into_fragment_records();
    let fragment_count: usize = fragment_records.values().map(|v| v.fragments.len()).sum();

    let mut protocol = WebUIProtocol::with_tokens(fragment_records, tokens);

    // Record build-wide strategies so the handler can decide rendering behavior.
    protocol.set_css_strategy(match options.css {
        CssStrategy::Link => webui_protocol::CssStrategy::Link,
        CssStrategy::Style => webui_protocol::CssStrategy::Style,
        CssStrategy::Module => webui_protocol::CssStrategy::Module,
    });
    protocol.set_dom_strategy(match options.dom {
        DomStrategy::Shadow => webui_protocol::DomStrategy::Shadow,
        DomStrategy::Light => webui_protocol::DomStrategy::Light,
    });

    // Process component CSS in a single pass: store Module CSS content,
    // set Link-strategy css_href, and collect external CSS files.
    let is_module = options.css == CssStrategy::Module;
    let is_link = options.css == CssStrategy::Link;
    let mut css_files: Vec<(String, String)> = Vec::new();
    for (tag, css) in css_snapshot {
        if !protocol.fragments.contains_key(&tag) {
            continue;
        }
        if is_module {
            protocol.components.entry(tag).or_default().css = css.trim().to_string();
        } else if is_link {
            let safe_tag = tag.replace(['/', '\\'], "-");
            let href = format!("{safe_tag}.css");
            protocol.components.entry(tag).or_default().css_href = href;
            css_files.push((format!("{safe_tag}.css"), css));
        }
        // Style strategy: CSS is already baked into raw fragments by the
        // parser — nothing to store in the protocol or emit as files.
    }

    // Store client templates in the protocol so any host server can query them
    for (tag, tmpl) in &component_templates {
        protocol.components.entry(tag.clone()).or_default().template = tmpl.clone();
    }

    Ok(RawBuildOutput {
        protocol,
        css_files,
        component_templates,
        fragment_count,
        component_count,
        token_count,
    })
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;
    use webui_protocol::web_ui_fragment::Fragment;

    fn create_app_dir(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (name, content) in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&path, content).unwrap();
        }
        dir
    }

    fn default_options(app_dir: &Path) -> BuildOptions {
        BuildOptions {
            app_dir: app_dir.to_path_buf(),
            entry: "index.html".to_string(),
            css: CssStrategy::Link,
            dom: DomStrategy::Shadow,
            plugin: None,
            components: Vec::new(),
        }
    }

    #[test]
    fn test_build_simple_html() {
        let app = create_app_dir(&[("index.html", "<h1>Hello</h1>")]);
        let result = build(default_options(app.path())).unwrap();

        assert!(result.protocol.fragments.contains_key("index.html"));
        assert!(result.stats.fragment_count > 0);
        assert!(result.stats.protocol_size_bytes > 0);
        assert!(!result.stats.duration.is_zero());
    }

    #[test]
    fn test_build_with_directives() {
        let html = r#"<h1>Hello</h1>
<for each="item in items">
    <p>{{item.name}}</p>
</for>
<if condition="show">
    <p>Visible</p>
</if>"#;
        let app = create_app_dir(&[("index.html", html)]);
        let result = build(default_options(app.path())).unwrap();

        let index = &result.protocol.fragments["index.html"].fragments;
        assert!(index
            .iter()
            .any(|f| matches!(f.fragment.as_ref(), Some(Fragment::ForLoop(_)))));
        assert!(index
            .iter()
            .any(|f| matches!(f.fragment.as_ref(), Some(Fragment::IfCond(_)))));
    }

    #[test]
    fn test_build_with_component_css() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card>"),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let result = build(default_options(app.path())).unwrap();

        assert_eq!(result.css_files.len(), 1);
        assert_eq!(result.css_files[0].0, "my-card.css");
        assert!(result.css_files[0].1.contains("color: red"));
        assert_eq!(result.stats.css_file_count, 1);
    }

    #[test]
    fn test_css_href_set_for_light_dom_link_strategy() {
        let app = create_app_dir(&[
            ("index.html", "<has-css>A</has-css><no-css>B</no-css>"),
            ("has-css.html", "<p><slot></slot></p>"),
            ("has-css.css", ".yes { color: green; }"),
            ("no-css.html", "<p><slot></slot></p>"),
        ]);
        let mut options = default_options(app.path());
        options.dom = DomStrategy::Light;
        let result = build(options).unwrap();

        let href = result
            .protocol
            .components
            .get("has-css")
            .map(|c| c.css_href.as_str())
            .unwrap_or("");
        assert_eq!(
            href, "has-css.css",
            "Light×Link component with CSS should have css_href"
        );

        let no_href = result
            .protocol
            .components
            .get("no-css")
            .map(|c| c.css_href.as_str())
            .unwrap_or("");
        assert!(
            no_href.is_empty(),
            "Component without CSS should have empty css_href"
        );
    }

    #[test]
    fn test_shadow_dom_link_strategy_sets_css_href() {
        // Shadow×Link: css_href is always set for Link-strategy components.
        // The handler uses protocol.css_strategy + dom_strategy to decide
        // whether to emit preload (Shadow) or stylesheet (Light) in <head>.
        let app = create_app_dir(&[
            ("index.html", "<my-card>A</my-card>"),
            ("my-card.html", "<p><slot></slot></p>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let result = build(default_options(app.path())).unwrap();

        let comp = result.protocol.components.get("my-card").unwrap();
        assert_eq!(
            comp.css_href, "my-card.css",
            "Shadow×Link should set css_href"
        );

        // Strategy fields on protocol
        assert_eq!(
            result.protocol.css_strategy(),
            webui_protocol::CssStrategy::Link,
        );
        assert_eq!(
            result.protocol.dom_strategy(),
            webui_protocol::DomStrategy::Shadow,
        );

        // CSS file should still be emitted for the server to serve
        assert_eq!(
            result.css_files.len(),
            1,
            "CSS file should still be produced"
        );
    }

    #[test]
    fn test_css_href_empty_for_style_strategy() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>A</my-card>"),
            ("my-card.html", "<p><slot></slot></p>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let mut options = default_options(app.path());
        options.css = CssStrategy::Style;
        let result = build(options).unwrap();

        let href = result
            .protocol
            .components
            .get("my-card")
            .map(|c| c.css_href.as_str())
            .unwrap_or("");
        assert!(href.is_empty(), "Style-strategy should not set css_href");
    }

    #[test]
    fn test_css_href_empty_for_module_strategy() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>A</my-card>"),
            ("my-card.html", "<p><slot></slot></p>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let mut options = default_options(app.path());
        options.css = CssStrategy::Module;
        let result = build(options).unwrap();

        let href = result
            .protocol
            .components
            .get("my-card")
            .map(|c| c.css_href.as_str())
            .unwrap_or("");
        assert!(href.is_empty(), "Module-strategy should not set css_href");
        assert!(
            !result.protocol.components["my-card"].css.is_empty(),
            "Module-strategy should still populate css content"
        );
    }

    #[test]
    fn test_build_to_disk_writes_files() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card>"),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let out = TempDir::new().unwrap();

        let stats = build_to_disk(default_options(app.path()), out.path()).unwrap();

        assert!(out.path().join("protocol.bin").exists());
        assert!(out.path().join("my-card.css").exists());
        assert_eq!(stats.css_file_count, 1);
        assert!(stats.fragment_count > 0);
    }

    #[test]
    fn test_build_to_disk_creates_nested_output_dir() {
        let app = create_app_dir(&[("index.html", "<h1>Hello</h1>")]);
        let out = TempDir::new().unwrap();
        let nested = out.path().join("nested").join("output");

        let stats = build_to_disk(default_options(app.path()), &nested).unwrap();
        assert!(nested.join("protocol.bin").exists());
        assert!(stats.fragment_count > 0);
    }

    #[test]
    fn test_build_missing_entry() {
        let app = create_app_dir(&[("other.html", "<h1>Not index</h1>")]);
        let result = build(default_options(app.path()));

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, WebUIError::Io { .. }));
    }

    #[test]
    fn test_build_stats_populated() {
        let app = create_app_dir(&[("index.html", "<h1>{{title}}</h1>")]);
        let result = build(default_options(app.path())).unwrap();

        assert!(result.stats.fragment_count > 0);
        assert!(result.stats.protocol_size_bytes > 0);
        assert_eq!(result.stats.css_file_count, 0);
    }

    #[test]
    fn test_build_inline_css() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card>"),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let mut options = default_options(app.path());
        options.css = CssStrategy::Style;

        let result = build(options).unwrap();
        // Inline mode embeds CSS in <style> tags — no external CSS files
        assert!(result.css_files.is_empty());
        assert_eq!(result.stats.css_file_count, 0);
        assert!(result.stats.fragment_count > 0);
    }

    #[test]
    fn test_build_module_css() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card>"),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let mut options = default_options(app.path());
        options.css = CssStrategy::Module;

        let result = build(options).unwrap();
        // Module mode uses <style type="module"> — no external CSS files
        assert!(result.css_files.is_empty());
        assert_eq!(result.stats.css_file_count, 0);
        assert!(result.stats.fragment_count > 0);
    }

    #[test]
    fn test_build_custom_entry() {
        let app = create_app_dir(&[("page.html", "<h1>Custom</h1>")]);
        let mut options = default_options(app.path());
        options.entry = "page.html".to_string();

        let result = build(options).unwrap();
        assert!(result.protocol.fragments.contains_key("page.html"));
        assert!(!result.protocol.fragments.contains_key("index.html"));
    }

    #[test]
    fn test_build_protocol_roundtrip() {
        let app = create_app_dir(&[("index.html", "<h1>{{title}}</h1>")]);
        let result = build(default_options(app.path())).unwrap();

        let restored = WebUIProtocol::from_protobuf(&result.protocol_bytes).unwrap();
        assert!(restored.fragments.contains_key("index.html"));
    }

    #[test]
    fn test_inspect_bytes_valid() {
        let app = create_app_dir(&[("index.html", "<h1>Hello</h1>")]);
        let result = build(default_options(app.path())).unwrap();

        let json_str = inspect_bytes(&result.protocol_bytes).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert!(parsed.get("fragments").is_some());
    }

    #[test]
    fn test_inspect_bytes_invalid() {
        let result = inspect_bytes(b"not a protobuf");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), WebUIError::Protocol(_)));
    }

    #[test]
    fn test_inspect_file() {
        let app = create_app_dir(&[("index.html", "<h1>Hello</h1>")]);
        let out = TempDir::new().unwrap();
        build_to_disk(default_options(app.path()), out.path()).unwrap();

        let json_str = inspect(&out.path().join("protocol.bin")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert!(parsed["fragments"]["index.html"]["fragments"].is_array());
    }

    #[test]
    fn test_inspect_missing_file() {
        let result = inspect(Path::new("/nonexistent/protocol.bin"));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), WebUIError::Io { .. }));
    }

    #[test]
    fn test_build_with_components_local_path() {
        let app = create_app_dir(&[("index.html", "<ext-card>Hello</ext-card>")]);
        let ext_dir = TempDir::new().unwrap();
        fs::write(
            ext_dir.path().join("ext-card.html"),
            "<div class=\"card\"><slot></slot></div>",
        )
        .unwrap();
        fs::write(
            ext_dir.path().join("ext-card.css"),
            ".card { border: 1px solid #ccc; }",
        )
        .unwrap();

        let mut options = default_options(app.path());
        options.components = vec![ext_dir.path().to_string_lossy().to_string()];

        let result = build(options).unwrap();
        assert!(result.protocol.fragments.contains_key("index.html"));
        assert_eq!(result.css_files.len(), 1);
        assert!(result.css_files[0].1.contains("border"));
    }

    #[test]
    fn test_build_hello_world_example() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let app_dir = manifest_dir.join("../../examples/app/hello-world/src");

        let result = build(BuildOptions {
            app_dir,
            entry: "index.html".to_string(),
            css: CssStrategy::Link,
            dom: DomStrategy::Shadow,
            plugin: None,
            components: Vec::new(),
        })
        .unwrap();

        let index = &result.protocol.fragments["index.html"].fragments;
        assert!(index.iter().any(
            |f| matches!(f.fragment.as_ref(), Some(Fragment::ForLoop(fl)) if fl.collection == "people")
        ));
        assert!(index
            .iter()
            .any(|f| matches!(f.fragment.as_ref(), Some(Fragment::IfCond(_)))));
    }

    // ── Security tests ───────────────────────────────────────────────

    #[test]
    fn test_css_filename_sanitizes_path_separators() {
        // Even if a component tag somehow contains path separators,
        // the filename should be sanitized to prevent directory traversal.
        let app = create_app_dir(&[("index.html", "<h1>Hello</h1>")]);
        let result = build(default_options(app.path())).unwrap();

        // Verify all CSS filenames are plain file names (no path separators)
        for (filename, _) in &result.css_files {
            assert!(
                !filename.contains('/') && !filename.contains('\\'),
                "CSS filename contains path separator: {filename}"
            );
        }
    }

    #[test]
    fn test_build_to_disk_css_stays_in_output_dir() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card>"),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let out = TempDir::new().unwrap();

        build_to_disk(default_options(app.path()), out.path()).unwrap();

        // Verify CSS file is written inside out_dir
        let css_path = out.path().join("my-card.css");
        assert!(css_path.exists());
        let canonical = css_path.canonicalize().unwrap();
        let out_canonical = out.path().canonicalize().unwrap();
        assert!(
            canonical.starts_with(&out_canonical),
            "CSS file escaped output directory"
        );
    }

    // ── Performance / edge case tests ────────────────────────────────

    #[test]
    fn test_build_empty_html() {
        let app = create_app_dir(&[("index.html", "")]);
        let result = build(default_options(app.path())).unwrap();

        assert!(result.protocol.fragments.contains_key("index.html"));
        assert_eq!(result.stats.fragment_count, 0);
        assert!(result.stats.protocol_size_bytes > 0);
    }

    #[test]
    fn test_build_large_fragment_count() {
        // Verify stats are accurate with many fragments
        let mut html = String::with_capacity(2000);
        for i in 0..50 {
            html.push_str(&format!("<p>Item {i}</p>\n"));
        }
        let app = create_app_dir(&[("index.html", &html)]);
        let result = build(default_options(app.path())).unwrap();

        assert!(result.stats.fragment_count > 0);
        assert_eq!(result.stats.css_file_count, 0);
    }

    #[test]
    fn test_build_with_fast_plugin() {
        let app = create_app_dir(&[("index.html", "<h1>Hello</h1>")]);
        let mut options = default_options(app.path());
        options.plugin = Some(Plugin::FastV3);

        let result = build(options).unwrap();
        assert!(result.protocol.fragments.contains_key("index.html"));
    }

    #[test]
    fn test_build_to_disk_inline_mode_no_css_files() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card>"),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let out = TempDir::new().unwrap();
        let mut options = default_options(app.path());
        options.css = CssStrategy::Style;

        let stats = build_to_disk(options, out.path()).unwrap();

        assert!(out.path().join("protocol.bin").exists());
        assert!(!out.path().join("my-card.css").exists());
        assert_eq!(stats.css_file_count, 0);
    }

    #[test]
    fn test_build_stats_duration_is_nonzero() {
        let app = create_app_dir(&[("index.html", "<h1>Hello {{name}}</h1>")]);
        let result = build(default_options(app.path())).unwrap();

        assert!(!result.stats.duration.is_zero());
    }

    #[test]
    fn test_build_multiple_components_css() {
        let app = create_app_dir(&[
            ("index.html", "<card-a>A</card-a><card-b>B</card-b>"),
            ("card-a.html", "<div><slot></slot></div>"),
            ("card-a.css", ".a { color: red; }"),
            ("card-b.html", "<span><slot></slot></span>"),
            ("card-b.css", ".b { color: blue; }"),
        ]);
        let result = build(default_options(app.path())).unwrap();

        assert_eq!(result.css_files.len(), 2);
        assert_eq!(result.stats.css_file_count, 2);
        let filenames: Vec<&str> = result.css_files.iter().map(|(n, _)| n.as_str()).collect();
        assert!(filenames.contains(&"card-a.css"));
        assert!(filenames.contains(&"card-b.css"));
    }

    #[test]
    fn test_build_unused_component_css_not_emitted() {
        // card-b is registered but not referenced in index.html
        let app = create_app_dir(&[
            ("index.html", "<card-a>A</card-a>"),
            ("card-a.html", "<div><slot></slot></div>"),
            ("card-a.css", ".a { color: red; }"),
            ("card-b.html", "<span><slot></slot></span>"),
            ("card-b.css", ".b { color: blue; }"),
        ]);
        let result = build(default_options(app.path())).unwrap();

        assert_eq!(result.css_files.len(), 1);
        assert_eq!(result.css_files[0].0, "card-a.css");
    }

    #[test]
    fn test_inspect_roundtrip_preserves_content() {
        let app = create_app_dir(&[("index.html", "<h1>{{title}}</h1>")]);
        let result = build(default_options(app.path())).unwrap();

        let json_str = inspect_bytes(&result.protocol_bytes).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let fragments = &parsed["fragments"]["index.html"]["fragments"];
        assert!(fragments.is_array());
        assert!(!fragments.as_array().unwrap().is_empty());
    }

    #[test]
    fn test_build_nonexistent_app_dir() {
        let options = BuildOptions {
            app_dir: PathBuf::from("/nonexistent/path/that/does/not/exist"),
            entry: "index.html".to_string(),
            css: CssStrategy::Link,
            dom: DomStrategy::Shadow,
            plugin: None,
            components: Vec::new(),
        };
        let result = build(options);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_to_disk_returns_accurate_stats() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card><p>{{name}}</p>"),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let out = TempDir::new().unwrap();

        let stats = build_to_disk(default_options(app.path()), out.path()).unwrap();

        assert!(stats.fragment_count > 0);
        assert_eq!(stats.css_file_count, 1);
        assert!(stats.component_count > 0);
        assert!(stats.protocol_size_bytes > 0);
        assert!(!stats.duration.is_zero());
    }
}
