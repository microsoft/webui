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
use webui_handler::{ResponseWriter, WebUIHandler};
use webui_protocol::WebUIProtocol;

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
pub fn render(
    protocol_data: Buffer,
    state_json: String,
    on_chunk: Function<String, ()>,
    plugin: Option<String>,
) -> napi::Result<()> {
    let protocol = WebUIProtocol::from_protobuf(&protocol_data)
        .map_err(|e| NapiError::from_reason(format!("Protocol decode error: {e}")))?;

    let state: Value = serde_json::from_str(&state_json)
        .map_err(|e| NapiError::from_reason(format!("State JSON error: {e}")))?;

    let mut writer = CallbackWriter::new(&on_chunk);
    let mut handler = match plugin.as_deref() {
        Some("fast") => WebUIHandler::with_plugin(Box::new(FastHydrationPlugin::new())),
        Some(unknown) => {
            return Err(NapiError::from_reason(format!("Unknown plugin: {unknown}")));
        }
        None => WebUIHandler::new(),
    };
    handler
        .render(&protocol, &state, &mut writer)
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
        let protocol = WebUIProtocol::new(parser.into_fragment_records());
        protocol.to_protobuf().expect("protobuf encode failed")
    }

    /// Helper: render protocol bytes + state, collecting output into a String.
    fn render_to_string(protocol_bytes: &[u8], state_json: &str) -> Result<String, String> {
        let protocol = WebUIProtocol::from_protobuf(protocol_bytes).map_err(|e| e.to_string())?;
        let state: Value = serde_json::from_str(state_json).map_err(|e| e.to_string())?;

        let mut output = String::with_capacity(1024);
        let mut handler = WebUIHandler::new();

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
            .render(&protocol, &state, &mut writer)
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
}
