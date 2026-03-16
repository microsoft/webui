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
use webui_handler::plugin::FastHydrationPlugin;
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
    /// Parser plugin (e.g., "fast").
    pub plugin: Option<String>,
    /// Additional component sources (npm packages or local paths).
    pub components: Option<Vec<String>>,
}

/// Build a WebUI application from an app directory.
///
/// Returns the compiled protocol bytes, CSS files, and build statistics.
#[napi]
pub fn build(options: JsBuildOptions) -> napi::Result<JsBuildResult> {
    let css = match options.css.as_deref() {
        Some("style") => webui::CssStrategy::Style,
        Some("link") | None => webui::CssStrategy::Link,
        Some(unknown) => {
            return Err(NapiError::from_reason(format!(
                "Unknown CSS mode: {unknown}. Use \"link\" or \"style\"."
            )));
        }
    };

    let build_options = webui::BuildOptions {
        app_dir: std::path::PathBuf::from(&options.app_dir),
        entry: options.entry.unwrap_or_else(|| "index.html".to_string()),
        css,
        plugin: options.plugin,
        components: options.components.unwrap_or_default(),
    };

    let result = webui::build(build_options)
        .map_err(|e| NapiError::from_reason(format!("Build error: {e}")))?;

    // Flatten css_files into alternating [filename, content, ...] for JS interop
    let css_files: Vec<String> = result
        .css_files
        .into_iter()
        .flat_map(|(name, content)| [name, content])
        .collect();

    Ok(JsBuildResult {
        protocol: Buffer::from(result.protocol_bytes),
        css_files,
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

/// Inspect protocol bytes and return a JSON representation.
#[napi]
pub fn inspect(protocol_data: Buffer) -> napi::Result<String> {
    webui::inspect_bytes(&protocol_data)
        .map_err(|e| NapiError::from_reason(format!("Inspect error: {e}")))
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
/// * `plugin` — Optional plugin identifier (e.g., `"fast"`).
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
    let handler = match plugin.as_deref() {
        Some("fast") => WebUIHandler::with_plugin(|| Box::new(FastHydrationPlugin::new())),
        Some(unknown) => {
            return Err(NapiError::from_reason(format!("Unknown plugin: {unknown}")));
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

    // ── Tests for build() and inspect() napi exports ─────────────────

    #[test]
    fn test_build_simple_app() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<h1>Hello</h1>").unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: None,
            css: None,
            plugin: None,
            components: None,
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
            plugin: None,
            components: None,
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
            plugin: None,
            components: None,
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
            plugin: None,
            components: None,
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
            plugin: None,
            components: None,
        };

        let result = build(options).unwrap();
        // css_files is flattened: [filename, content, filename, content, ...]
        assert_eq!(result.css_files.len(), 2);
        assert_eq!(result.css_files[0], "my-card.css");
        assert!(result.css_files[1].contains("color: red"));
        assert_eq!(result.stats.css_file_count, 1);
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
}
