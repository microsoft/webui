//! Node.js native addon for the WebUI framework via napi-rs.
//!
//! Provides high-performance server-side rendering by compiling the Rust
//! WebUI handler directly into a `.node` native addon — no C ABI intermediary.
//!
//! ## Usage (from Node.js)
//!
//! ```js
//! // Load the native addon from the cargo build output
//! const mod = { exports: {} };
//! process.dlopen(mod, './target/release/libwebui_node.dylib');
//! const html = mod.exports.render('<h1>Hello, {{name}}!</h1>', '{"name": "WebUI"}');
//! // => "<h1>Hello, WebUI!</h1>"
//! ```

use napi::Error as NapiError;
use napi_derive::napi;
use serde_json::Value;
use webui_handler::{ResponseWriter, WebUIHandler};
use webui_parser::HtmlParser;
use webui_protocol::WebUIProtocol;

/// A simple string buffer for collecting rendered output.
struct StringWriter {
    content: String,
}

impl StringWriter {
    fn with_capacity(cap: usize) -> Self {
        Self {
            content: String::with_capacity(cap),
        }
    }
}

impl ResponseWriter for StringWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.content.push_str(content);
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

/// Parse an HTML template and render it with JSON state data.
///
/// This is the main entry point for Node.js consumers. It parses the template,
/// builds a protocol, and renders it with the provided state — all in one call.
#[napi]
pub fn render(html: String, data_json: String) -> napi::Result<String> {
    render_inner(&html, &data_json).map_err(|e| NapiError::from_reason(e.to_string()))
}

fn render_inner(html: &str, data_json: &str) -> Result<String, RenderError> {
    let mut parser = HtmlParser::new();
    parser
        .parse("index.html", html)
        .map_err(|e| RenderError::Parse(e.to_string()))?;

    let protocol = WebUIProtocol {
        fragments: parser.into_fragment_records(),
    };

    let state: Value =
        serde_json::from_str(data_json).map_err(|e| RenderError::State(e.to_string()))?;

    let initial_cap = html.len().max(1024);
    let mut writer = StringWriter::with_capacity(initial_cap);
    let handler = WebUIHandler::new();
    handler
        .render(&protocol, &state, &mut writer)
        .map_err(|e| RenderError::Render(e.to_string()))?;

    Ok(writer.content)
}

/// Errors from the render pipeline.
#[derive(Debug, PartialEq)]
enum RenderError {
    Parse(String),
    State(String),
    Render(String),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderError::Parse(msg) => write!(f, "HTML parse error: {msg}"),
            RenderError::State(msg) => write!(f, "State JSON error: {msg}"),
            RenderError::Render(msg) => write!(f, "Render error: {msg}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_passthrough() {
        let result = render_inner("<p>Hello</p>", "{}");
        assert_eq!(result.as_deref(), Ok("<p>Hello</p>"));
    }

    #[test]
    fn test_signal_substitution() {
        let result = render_inner("Hello, {{name}}!", r#"{"name": "WebUI"}"#);
        assert_eq!(result.as_deref(), Ok("Hello, WebUI!"));
    }

    #[test]
    fn test_for_loop() {
        let html = "<ul><for each=\"item in items\"><li>{{item}}</li></for></ul>";
        let state = r#"{"items": ["a", "b", "c"]}"#;
        let result = render_inner(html, state);
        assert_eq!(
            result.as_deref(),
            Ok("<ul><li>a</li><li>b</li><li>c</li></ul>")
        );
    }

    #[test]
    fn test_if_condition_true() {
        let html = "<if condition=\"show\"><p>Visible</p></if>";
        let result = render_inner(html, r#"{"show": true}"#);
        assert_eq!(result.as_deref(), Ok("<p>Visible</p>"));
    }

    #[test]
    fn test_if_condition_false() {
        let html = "<if condition=\"show\"><p>Hidden</p></if>";
        let result = render_inner(html, r#"{"show": false}"#);
        assert_eq!(result.as_deref(), Ok(""));
    }

    #[test]
    fn test_html_escaping() {
        let html = "<div>{{content}}</div>";
        let state = r#"{"content": "<script>alert('xss')</script>"}"#;
        let result = render_inner(html, state).expect("render should succeed");
        assert!(!result.contains("<script>"));
        assert!(result.contains("&lt;script&gt;"));
    }

    #[test]
    fn test_raw_signal() {
        let html = "<div>{{{content}}}</div>";
        let state = r#"{"content": "<b>bold</b>"}"#;
        let result = render_inner(html, state);
        assert_eq!(result.as_deref(), Ok("<div><b>bold</b></div>"));
    }

    #[test]
    fn test_invalid_json() {
        let result = render_inner("<p>hi</p>", "NOT JSON");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("State JSON error"), "Unexpected error: {err}");
    }

    #[test]
    fn test_empty_state() {
        let result = render_inner("<p>static</p>", "{}");
        assert_eq!(result.as_deref(), Ok("<p>static</p>"));
    }

    #[test]
    fn test_nested_object_signal() {
        let html = "{{user.name}}";
        let state = r#"{"user": {"name": "Alice"}}"#;
        let result = render_inner(html, state);
        assert_eq!(result.as_deref(), Ok("Alice"));
    }
}
