// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Node.js native addon for the WebUI framework via napi-rs.
//!
//! Provides high-performance server-side rendering by compiling the Rust
//! WebUI handler directly into a `.node` native addon — no C ABI intermediary.
//!
//! The `render` function accepts pre-compiled protobuf protocol data (from
//! `webui build`) and streams rendered HTML fragments via a callback, enabling
//! true streaming SSR without buffering the entire response.
//!
//! ## Usage (from Node.js)
//!
//! ```js
//! import fs from 'node:fs';
//!
//! // Load the native addon
//! const mod = { exports: {} };
//! process.dlopen(mod, './target/release/libwebui_node.dylib');
//! const { render } = mod.exports;
//!
//! // Read pre-compiled protocol (from `webui build`)
//! const protocol = fs.readFileSync('./dist/protocol.bin');
//! const state = '{"name": "WebUI"}';
//!
//! // Stream rendered fragments
//! render(protocol, state, (chunk) => process.stdout.write(chunk));
//! ```

use napi::bindgen_prelude::{Buffer, Function};
use napi::Error as NapiError;
use napi_derive::napi;
use serde_json::Value;
use webui_handler::plugin::fast_v2::FastV2HydrationPlugin;
use webui_handler::plugin::fast_v3::FastV3HydrationPlugin;
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::{RenderOptions, ResponseWriter, WebUIHandler};
use webui_protocol::WebUIProtocol;

/// Build statistics returned from the build function.
#[napi(object)]
pub struct JsBuildStats {
    /// Build duration in milliseconds.
    pub duration_ms: f64,
    /// Total number of protocol fragments.
    pub fragment_count: u32,
    /// Number of registered components.
    pub component_count: u32,
    /// Number of CSS files produced.
    pub css_file_count: u32,
    /// Size of the serialized protocol in bytes.
    pub protocol_size_bytes: u32,
    /// Number of unique CSS tokens discovered.
    pub token_count: u32,
}

/// Result of a successful build operation.
#[napi(object)]
pub struct JsBuildResult {
    /// Serialized protocol (protobuf binary).
    pub protocol: Buffer,
    /// CSS files as alternating [filename, content, filename, content, ...].
    pub css_files: Vec<String>,
    /// Static component asset files as alternating [filename, content, filename, content, ...].
    pub component_asset_files: Vec<String>,
    /// Non-fatal build advisories (plain text), e.g. CSS tokens used only with a
    /// literal `var()` fallback and absent from every theme.
    pub warnings: Vec<String>,
    /// Build statistics.
    pub stats: JsBuildStats,
}

/// Build options for the webui build API.
#[napi(object)]
pub struct JsBuildOptions {
    /// Path to the application folder containing templates.
    pub app_dir: String,
    /// Entry HTML file name (defaults to "index.html").
    pub entry: Option<String>,
    /// CSS mode: "link" (default) or "style".
    pub css: Option<String>,
    /// DOM strategy for component rendering: "shadow" (default) or "light".
    pub dom: Option<String>,
    /// Plugin identifier (see crate documentation for available identifiers).
    pub plugin: Option<String>,
    /// Additional component sources (npm packages or local paths).
    pub components: Option<Vec<String>>,
    /// Root component tags emitted as static `.webui.js` ESM assets.
    pub component_asset_roots: Option<Vec<String>>,
    /// Link-mode CSS filename template using [name], [hash], [ext].
    pub css_file_name_template: Option<String>,
    /// Optional base URL/path prefix for Link-mode css hrefs.
    pub css_public_base: Option<String>,
    /// Legal comment handling: "inline" (default) or "none".
    pub legal_comments: Option<String>,
    /// Design token theme: a JSON file path or npm package name.
    pub theme: Option<String>,
}

/// Build a WebUI application from an app directory.
///
/// Returns the compiled protocol bytes, CSS files, and build statistics.
#[napi]
#[allow(clippy::cast_possible_truncation)] // stats are bounded by component/file counts
pub fn build(options: JsBuildOptions) -> napi::Result<JsBuildResult> {
    let css = options
        .css
        .map(|s| s.parse::<webui::CssStrategy>())
        .transpose()
        .map_err(NapiError::from_reason)?
        .unwrap_or_default();

    let dom = options
        .dom
        .map(|s| s.parse::<webui::DomStrategy>())
        .transpose()
        .map_err(NapiError::from_reason)?
        .unwrap_or_default();

    let plugin = options
        .plugin
        .map(|s| s.parse::<webui::Plugin>())
        .transpose()
        .map_err(NapiError::from_reason)?;

    let legal_comments = options
        .legal_comments
        .map(|s| s.parse::<webui::LegalComments>())
        .transpose()
        .map_err(NapiError::from_reason)?
        .unwrap_or_default();

    let app_dir = std::path::PathBuf::from(&options.app_dir);
    let theme = options
        .theme
        .as_deref()
        .map(|theme| load_theme(theme, &app_dir))
        .transpose()?;

    let build_options = webui::BuildOptions {
        app_dir,
        entry: options.entry.unwrap_or_else(|| "index.html".to_string()),
        css,
        dom,
        plugin,
        components: options.components.unwrap_or_default(),
        component_asset_roots: options.component_asset_roots.unwrap_or_default(),
        css_file_name_template: options
            .css_file_name_template
            .unwrap_or_else(|| webui::DEFAULT_CSS_FILE_NAME_TEMPLATE.to_string()),
        css_public_base: options.css_public_base,
        legal_comments,
        theme,
    };

    let result = webui::build(build_options)
        .map_err(|e| NapiError::from_reason(format!("Build error: {}", e.chain_message())))?;

    // Flatten css_files into alternating [filename, content, ...] for JS interop
    let css_files: Vec<String> = result
        .css_files
        .into_iter()
        .flat_map(|(name, content)| [name, content])
        .collect();
    let component_asset_files: Vec<String> = result
        .component_asset_files
        .into_iter()
        .flat_map(|file| [file.name, file.content])
        .collect();
    let warnings: Vec<String> = result.warnings.iter().map(|d| d.to_string()).collect();

    Ok(JsBuildResult {
        protocol: Buffer::from(result.protocol_bytes),
        css_files,
        component_asset_files,
        warnings,
        stats: JsBuildStats {
            duration_ms: result.stats.duration.as_secs_f64() * 1000.0,
            fragment_count: result.stats.fragment_count as u32,
            component_count: result.stats.component_count as u32,
            css_file_count: result.stats.css_file_count as u32,
            protocol_size_bytes: result.stats.protocol_size_bytes as u32,
            token_count: result.stats.token_count as u32,
        },
    })
}

fn load_theme(theme: &str, search_root: &std::path::Path) -> napi::Result<webui::TokenFile> {
    let resolved = webui::resolve_theme_path(theme, search_root)
        .map_err(|e| NapiError::from_reason(format!("Theme resolution error: {e}")))?;
    webui::load_token_file(&resolved).map_err(|e| {
        NapiError::from_reason(format!("Theme load error for {}: {e}", resolved.display()))
    })
}

/// Inspect protocol bytes and return a JSON representation.
#[napi]
pub fn inspect(protocol_data: Buffer) -> napi::Result<String> {
    webui::inspect_bytes(&protocol_data)
        .map_err(|e| NapiError::from_reason(format!("Inspect error: {e}")))
}

/// Produce a complete JSON partial response for client-side navigation.
///
/// Combines application state, route templates, inventory, request path, and
/// matched route chain into a single JSON string:
/// `{"state":{...},"templates":[...],"inventory":"...","path":"...","chain":[...]}`.
///
/// Host servers return this directly — no assembly required.
#[napi]
pub fn render_partial(
    protocol_data: Buffer,
    state_json: String,
    entry_id: String,
    request_path: String,
    inventory_hex: String,
) -> napi::Result<String> {
    let protocol = WebUIProtocol::from_protobuf(&protocol_data)
        .map_err(|e| NapiError::from_reason(format!("Protocol decode error: {e}")))?;

    let state: serde_json::Value = serde_json::from_str(&state_json)
        .map_err(|e| NapiError::from_reason(format!("invalid state JSON: {e}")))?;

    // TODO: ProtocolIndex is created per-request here because the Node binding
    // receives the protocol fresh each call. Ideally the host should cache it
    // alongside the protocol for the lifetime of the server — it's deterministic
    // per protocol and avoids rebuilding the component index + route cache.
    let mut index = webui_handler::route_handler::ProtocolIndex::new(&protocol);

    let mut result = webui_handler::route_handler::render_partial(
        &protocol,
        &entry_id,
        &request_path,
        &inventory_hex,
        &mut index,
    )
    .map_err(|e| NapiError::from_reason(format!("render_partial failed: {e}")))?;
    if let Some(obj) = result.as_object_mut() {
        obj.insert("state".into(), state);
    }

    serde_json::to_string(&result)
        .map_err(|e| NapiError::from_reason(format!("JSON serialize error: {e}")))
}

#[napi]
pub fn render_component_templates(
    protocol_data: Buffer,
    component_tags_json: String,
    inventory_hex: String,
) -> napi::Result<String> {
    let protocol = WebUIProtocol::from_protobuf(&protocol_data)
        .map_err(|e| NapiError::from_reason(format!("Protocol decode error: {e}")))?;

    let tags: Vec<String> = serde_json::from_str(&component_tags_json)
        .map_err(|e| NapiError::from_reason(format!("invalid tags JSON: {e}")))?;
    let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();

    // Per-request index — see ProtocolIndex doc for caching guidance.
    let mut index = webui_handler::route_handler::ProtocolIndex::new(&protocol);

    let result = webui_handler::route_handler::render_component_templates(
        &protocol,
        &tag_refs,
        &inventory_hex,
        &mut index,
    )
    .map_err(|e| NapiError::from_reason(format!("render_component_templates failed: {e}")))?;

    serde_json::to_string(&result)
        .map_err(|e| NapiError::from_reason(format!("JSON serialize error: {e}")))
}

/// Extract the CSS token name list from a compiled protocol.
///
/// Returns the tokens as a JavaScript string array, preserving the original
/// order from the build step.
#[napi]
pub fn protocol_tokens(protocol_data: Buffer) -> napi::Result<Vec<String>> {
    let protocol = WebUIProtocol::from_protobuf(&protocol_data)
        .map_err(|e| NapiError::from_reason(format!("Protocol decode error: {e}")))?;

    Ok(protocol.tokens)
}

/// A writer that streams each rendered fragment to a JS callback.
struct CallbackWriter<'a, 'env> {
    callback: &'a Function<'env, String, ()>,
    error: Option<String>,
}

impl<'a, 'env> CallbackWriter<'a, 'env> {
    fn new(callback: &'a Function<'env, String, ()>) -> Self {
        Self {
            callback,
            error: None,
        }
    }
}

impl ResponseWriter for CallbackWriter<'_, '_> {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        if self.error.is_some() {
            return Ok(());
        }
        if let Err(e) = self.callback.call(content.to_owned()) {
            // Ignore "Value is not undefined" errors from callbacks that
            // return non-void (e.g. res.write() returns a boolean).
            let msg = format!("{e}");
            if !msg.contains("Value is not undefined") {
                self.error = Some(msg);
            }
        }
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

/// Render a pre-compiled WebUI protocol with JSON state, streaming each
/// fragment to the provided callback.
///
/// # Arguments
///
/// * `protocol_data` — Protobuf binary from `webui build` (zero-copy Buffer).
/// * `state_json` — JSON string with the render state.
/// * `on_chunk` — Called with each rendered HTML fragment as it is produced.
/// * `plugin` — Optional plugin identifier (see crate documentation for available identifiers).
#[napi]
#[allow(clippy::too_many_arguments)]
pub fn render(
    protocol_data: Buffer,
    state_json: String,
    entry: String,
    request_path: String,
    on_chunk: Function<String, ()>,
    plugin: Option<String>,
) -> napi::Result<()> {
    let protocol = WebUIProtocol::from_protobuf(&protocol_data)
        .map_err(|e| NapiError::from_reason(format!("Protocol decode error: {e}")))?;

    let state: Value = serde_json::from_str(&state_json)
        .map_err(|e| NapiError::from_reason(format!("State JSON error: {e}")))?;

    let mut writer = CallbackWriter::new(&on_chunk);
    let plugin = plugin
        .map(|s| s.parse::<webui::Plugin>())
        .transpose()
        .map_err(NapiError::from_reason)?;
    let handler = match plugin {
        Some(webui::Plugin::Fast | webui::Plugin::FastV2) => {
            WebUIHandler::with_plugin(|| Box::new(FastV2HydrationPlugin::new()))
        }
        Some(webui::Plugin::FastV3) => {
            WebUIHandler::with_plugin(|| Box::new(FastV3HydrationPlugin::new()))
        }
        Some(webui::Plugin::WebUI) => {
            WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()))
        }
        None => WebUIHandler::new(),
    };
    handler
        .render(
            &protocol,
            &state,
            &RenderOptions::new(&entry, &request_path),
            &mut writer,
        )
        .map_err(|e| NapiError::from_reason(format!("Render error: {e}")))?;

    if let Some(err) = writer.error {
        return Err(NapiError::from_reason(format!("Callback error: {err}")));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use webui_parser::HtmlParser;

    /// Helper: parse HTML into protobuf bytes for testing.
    fn build_protocol(html: &str) -> Vec<u8> {
        let mut parser = HtmlParser::new();
        parser.parse("index.html", html).expect("parse failed");
        let tokens = parser.take_tokens();
        let protocol = WebUIProtocol::with_tokens(parser.into_fragment_records(), tokens);
        protocol.to_protobuf().expect("protobuf encode failed")
    }

    /// Helper: render protocol bytes + state, collecting output into a String.
    fn render_to_string(protocol_bytes: &[u8], state_json: &str) -> Result<String, String> {
        let protocol = WebUIProtocol::from_protobuf(protocol_bytes).map_err(|e| e.to_string())?;
        let state: Value = serde_json::from_str(state_json).map_err(|e| e.to_string())?;

        let mut output = String::with_capacity(1024);
        let handler = WebUIHandler::new();

        struct StringWriter<'a> {
            output: &'a mut String,
        }
        impl ResponseWriter for StringWriter<'_> {
            fn write(&mut self, content: &str) -> webui_handler::Result<()> {
                self.output.push_str(content);
                Ok(())
            }
            fn end(&mut self) -> webui_handler::Result<()> {
                Ok(())
            }
        }

        let mut writer = StringWriter {
            output: &mut output,
        };
        handler
            .render(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .map_err(|e| e.to_string())?;
        Ok(output)
    }

    #[test]
    fn test_simple_passthrough() {
        let proto = build_protocol("<p>Hello</p>");
        let result = render_to_string(&proto, "{}");
        assert_eq!(result.as_deref(), Ok("<p>Hello</p>"));
    }

    #[test]
    fn test_signal_substitution() {
        let proto = build_protocol("Hello, {{name}}!");
        let result = render_to_string(&proto, r#"{"name": "WebUI"}"#);
        assert_eq!(result.as_deref(), Ok("Hello, WebUI!"));
    }

    #[test]
    fn test_for_loop() {
        let proto = build_protocol("<ul><for each=\"item in items\"><li>{{item}}</li></for></ul>");
        let result = render_to_string(&proto, r#"{"items": ["a", "b", "c"]}"#);
        assert_eq!(
            result.as_deref(),
            Ok("<ul><li>a</li><li>b</li><li>c</li></ul>")
        );
    }

    #[test]
    fn test_if_condition_true() {
        let proto = build_protocol("<if condition=\"show\"><p>Visible</p></if>");
        let result = render_to_string(&proto, r#"{"show": true}"#);
        assert_eq!(result.as_deref(), Ok("<p>Visible</p>"));
    }

    #[test]
    fn test_if_condition_false() {
        let proto = build_protocol("<if condition=\"show\"><p>Hidden</p></if>");
        let result = render_to_string(&proto, r#"{"show": false}"#);
        assert_eq!(result.as_deref(), Ok(""));
    }

    #[test]
    fn test_html_escaping() {
        let proto = build_protocol("<div>{{content}}</div>");
        let state = r#"{"content": "<script>alert('xss')</script>"}"#;
        let result = render_to_string(&proto, state).expect("render should succeed");
        assert!(!result.contains("<script>"));
        assert!(result.contains("&lt;script&gt;"));
    }

    #[test]
    fn test_raw_signal() {
        let proto = build_protocol("<div>{{{content}}}</div>");
        let result = render_to_string(&proto, r#"{"content": "<b>bold</b>"}"#);
        assert_eq!(result.as_deref(), Ok("<div><b>bold</b></div>"));
    }

    #[test]
    fn test_invalid_json() {
        let proto = build_protocol("<p>hi</p>");
        let result = render_to_string(&proto, "NOT JSON");
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_state() {
        let proto = build_protocol("<p>static</p>");
        let result = render_to_string(&proto, "{}");
        assert_eq!(result.as_deref(), Ok("<p>static</p>"));
    }

    #[test]
    fn test_nested_object_signal() {
        let proto = build_protocol("{{user.name}}");
        let result = render_to_string(&proto, r#"{"user": {"name": "Alice"}}"#);
        assert_eq!(result.as_deref(), Ok("Alice"));
    }

    #[test]
    fn test_invalid_protobuf() {
        let result = render_to_string(&[0xFF, 0xFF, 0xFF], "{}");
        assert!(result.is_err());
    }

    /// Parse `html`, attach a sorted hydration `schema`, and encode to protobuf.
    fn build_projected_protocol(html: &str, schema: &[&str]) -> Vec<u8> {
        let mut parser = HtmlParser::new();
        parser.parse("index.html", html).expect("parse failed");
        let tokens = parser.take_tokens();
        let mut protocol = WebUIProtocol::with_tokens(parser.into_fragment_records(), tokens);
        protocol.hydration_schema = schema.iter().map(|s| (*s).to_string()).collect();
        protocol.to_protobuf().expect("protobuf encode failed")
    }

    /// Render protocol bytes with the WebUI hydration plugin so the `#webui-data`
    /// bootstrap block (and its projected state) is emitted — this mirrors the
    /// production `render(..., plugin = "webui")` path.
    fn render_with_webui_plugin(protocol_bytes: &[u8], state_json: &str) -> Result<String, String> {
        let protocol = WebUIProtocol::from_protobuf(protocol_bytes).map_err(|e| e.to_string())?;
        let state: Value = serde_json::from_str(state_json).map_err(|e| e.to_string())?;

        let mut output = String::with_capacity(1024);
        struct StringWriter<'a> {
            output: &'a mut String,
        }
        impl ResponseWriter for StringWriter<'_> {
            fn write(&mut self, content: &str) -> webui_handler::Result<()> {
                self.output.push_str(content);
                Ok(())
            }
            fn end(&mut self) -> webui_handler::Result<()> {
                Ok(())
            }
        }
        let mut writer = StringWriter {
            output: &mut output,
        };
        let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
        handler
            .render(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .map_err(|e| e.to_string())?;
        Ok(output)
    }

    #[test]
    fn render_projects_state_to_hydration_schema() {
        // Full document so the parser emits a `body_end` signal, which makes the
        // WebUI plugin emit the #webui-data bootstrap block.
        let bytes =
            build_projected_protocol("<html><body><p>{{kept}}</p></body></html>", &["kept"]);
        let out =
            render_with_webui_plugin(&bytes, r#"{"kept":"KEPT_VALUE","dropped":"DROPPED_VALUE"}"#)
                .expect("render should succeed");

        // Only the hydratable key reaches the bootstrap state block...
        assert!(
            out.contains(r#""kept":"KEPT_VALUE""#),
            "hydratable key missing from bootstrap state: {out}"
        );
        // ...the non-hydratable key is projected out entirely.
        assert!(
            !out.contains("DROPPED_VALUE"),
            "server-only value leaked: {out}"
        );
        assert!(
            !out.contains("dropped"),
            "server-only key name leaked: {out}"
        );
    }

    // ── Tests for build() and inspect() napi exports ─────────────────

    #[test]
    fn test_build_simple_app() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<h1>Hello</h1>").unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: None,
            css: None,
            dom: None,
            plugin: None,
            components: None,
            component_asset_roots: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: None,
            theme: None,
        };

        let result = build(options).unwrap();
        assert!(!result.protocol.is_empty());
        assert!(result.stats.fragment_count > 0);
        assert!(result.stats.protocol_size_bytes > 0);
        assert!(result.stats.duration_ms >= 0.0);
    }

    #[test]
    fn test_build_with_custom_entry() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("page.html"), "<p>Custom</p>").unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: Some("page.html".to_string()),
            css: None,
            dom: None,
            plugin: None,
            components: None,
            component_asset_roots: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: None,
            theme: None,
        };

        let result = build(options).unwrap();
        assert!(!result.protocol.is_empty());
    }

    #[test]
    fn test_build_missing_app_dir() {
        let options = JsBuildOptions {
            app_dir: "/nonexistent/path".to_string(),
            entry: None,
            css: None,
            dom: None,
            plugin: None,
            components: None,
            component_asset_roots: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: None,
            theme: None,
        };

        let result = build(options);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_invalid_css() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<h1>Hello</h1>").unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: None,
            css: Some("bogus".to_string()),
            dom: None,
            plugin: None,
            components: None,
            component_asset_roots: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: None,
            theme: None,
        };

        let result = build(options);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_with_components_css() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<my-card>Hello</my-card>").unwrap();
        std::fs::write(dir.path().join("my-card.html"), "<div><slot></slot></div>").unwrap();
        std::fs::write(dir.path().join("my-card.css"), ".card { color: red; }").unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: None,
            css: Some("link".to_string()),
            dom: None,
            plugin: None,
            components: None,
            component_asset_roots: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: None,
            theme: None,
        };

        let result = build(options).unwrap();
        // css_files is flattened: [filename, content, filename, content, ...]
        assert_eq!(result.css_files.len(), 2);
        assert_eq!(result.css_files[0], "my-card.css");
        assert!(result.css_files[1].contains("color: red"));
        assert_eq!(result.stats.css_file_count, 1);
    }

    #[test]
    fn test_build_with_theme_missing_token_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<my-card></my-card>").unwrap();
        std::fs::write(dir.path().join("my-card.html"), "<div>Card</div>").unwrap();
        std::fs::write(
            dir.path().join("my-card.css"),
            ":host { --token-a: red; --foo-bar: var(--token-a, var(--token-b, var(--token-c))); }",
        )
        .unwrap();
        let theme_path = dir.path().join("theme.json");
        std::fs::write(&theme_path, r#"{"themes":{"light":{"token-b":"green"}}}"#).unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: None,
            css: Some("link".to_string()),
            dom: None,
            plugin: None,
            components: None,
            component_asset_roots: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: None,
            theme: Some(theme_path.to_string_lossy().to_string()),
        };

        let Err(err) = build(options) else {
            panic!("missing theme token must fail");
        };
        let message = err.to_string();
        assert!(message.contains("missing-theme-token"), "msg: {message}");
        assert!(message.contains("--token-c"), "msg: {message}");
    }

    #[test]
    fn test_build_returns_component_asset_files() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<app-shell></app-shell>").unwrap();
        std::fs::write(dir.path().join("app-shell.html"), "<div></div>").unwrap();
        std::fs::write(dir.path().join("lazy-panel.html"), "<p>{{title}}</p>").unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: None,
            css: Some("link".to_string()),
            dom: None,
            plugin: Some("webui".to_string()),
            components: None,
            component_asset_roots: Some(vec!["lazy-panel".to_string()]),
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: None,
            theme: None,
        };

        let result = build(options).unwrap();

        assert_eq!(result.component_asset_files.len(), 2);
        assert_eq!(result.component_asset_files[0], "lazy-panel.webui.js");
        assert!(result.component_asset_files[1].contains("webui-component-asset"));
        assert!(result.component_asset_files[1].contains("export default asset;"));
    }

    #[test]
    fn test_build_legal_comments_none_strips_legal_css() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<my-card>Hello</my-card>").unwrap();
        std::fs::write(dir.path().join("my-card.html"), "<div><slot></slot></div>").unwrap();
        std::fs::write(
            dir.path().join("my-card.css"),
            "/*! @license MIT */ .card { color: red; }",
        )
        .unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: None,
            css: Some("link".to_string()),
            dom: None,
            plugin: None,
            components: None,
            component_asset_roots: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: Some("none".to_string()),
            theme: None,
        };

        let result = build(options).unwrap();
        assert_eq!(result.css_files[1], " .card { color: red; }");
    }

    #[test]
    fn test_build_invalid_legal_comments() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<h1>Hello</h1>").unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: None,
            css: None,
            dom: None,
            plugin: None,
            components: None,
            component_asset_roots: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: Some("linked".to_string()),
            theme: None,
        };

        let result = build(options);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_with_light_dom_omits_shadow_root_template() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<my-card>Hi</my-card>").unwrap();
        std::fs::write(dir.path().join("my-card.html"), "<div><slot></slot></div>").unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: None,
            css: None,
            dom: Some("light".to_string()),
            plugin: None,
            components: None,
            component_asset_roots: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: None,
            theme: None,
        };

        let result = build(options).unwrap();
        let json = inspect(result.protocol).unwrap();
        assert!(
            !json.contains("shadowrootmode"),
            "light DOM build should not emit shadowrootmode wrappers, got: {json}"
        );
    }

    #[test]
    fn test_build_with_shadow_dom_emits_shadow_root_template() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<my-card>Hi</my-card>").unwrap();
        std::fs::write(dir.path().join("my-card.html"), "<div><slot></slot></div>").unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: None,
            css: None,
            dom: Some("shadow".to_string()),
            plugin: None,
            components: None,
            component_asset_roots: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: None,
            theme: None,
        };

        let result = build(options).unwrap();
        let json = inspect(result.protocol).unwrap();
        assert!(
            json.contains("shadowrootmode"),
            "shadow DOM build should emit shadowrootmode wrappers, got: {json}"
        );
    }

    #[test]
    fn test_build_invalid_dom() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<h1>Hello</h1>").unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: None,
            css: None,
            dom: Some("bogus".to_string()),
            plugin: None,
            components: None,
            component_asset_roots: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: None,
            theme: None,
        };

        let result = build(options);
        assert!(result.is_err());
    }

    #[test]
    fn test_inspect_valid_protocol() {
        let proto = build_protocol("<h1>Hello {{name}}</h1>");
        let json = inspect(napi::bindgen_prelude::Buffer::from(proto)).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("fragments").is_some());
    }

    #[test]
    fn test_inspect_invalid_protocol() {
        let result = inspect(napi::bindgen_prelude::Buffer::from(vec![0xFF, 0xFF]));
        assert!(result.is_err());
    }

    // ── Tests for protocol_tokens napi export ────────────────────────

    #[test]
    fn test_protocol_tokens_empty() {
        let proto = build_protocol("<p>Hello</p>");
        let tokens = protocol_tokens(napi::bindgen_prelude::Buffer::from(proto)).unwrap();
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_protocol_tokens_returns_parsed_tokens() {
        // Build from a protocol that has CSS tokens via with_tokens constructor.
        let mut parser = HtmlParser::new();
        parser.parse("index.html", "<p>Hi</p>").expect("parse");
        let protocol = WebUIProtocol::with_tokens(
            parser.into_fragment_records(),
            vec![
                "colorBrandBackground".to_string(),
                "fontSizeBase300".to_string(),
            ],
        );
        let proto = protocol.to_protobuf().expect("encode");
        let tokens = protocol_tokens(napi::bindgen_prelude::Buffer::from(proto)).unwrap();
        assert_eq!(tokens, vec!["colorBrandBackground", "fontSizeBase300"]);
    }

    #[test]
    fn test_protocol_tokens_invalid_protobuf() {
        let result = protocol_tokens(napi::bindgen_prelude::Buffer::from(vec![0xFF, 0xFF]));
        assert!(result.is_err());
    }
}
