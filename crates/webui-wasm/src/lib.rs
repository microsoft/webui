// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! WebAssembly bindings for the WebUI framework.
//!
//! This crate can be built as three WASM variants:
//! - **handler** - render pre-built protocol bytes with state.
//! - **parser** - build protocol bytes from virtual files.
//! - **all** - parser plus handler exports for playground-style live preview.

mod error;

#[cfg(all(not(feature = "handler"), not(feature = "parser")))]
compile_error!("microsoft-webui-wasm requires at least one of the `handler` or `parser` features");

#[cfg(feature = "handler")]
mod handler;
#[cfg(feature = "parser")]
mod parser;

#[cfg(feature = "handler")]
pub use handler::Protocol;
#[cfg(feature = "parser")]
pub use parser::build_protocol;

#[cfg(all(test, feature = "handler", feature = "parser"))]
mod tests {
    use super::*;
    use crate::error::WasmError;
    use std::collections::HashMap;
    use webui_protocol::WebUIProtocol;

    fn render_files_for_test(
        files: &HashMap<String, String>,
        state_json: &str,
        entry: &str,
        request_path: &str,
    ) -> Result<String, WasmError> {
        let protocol = parser::parse_to_protocol(files, entry, &[])?;
        handler::render_protocol_to_string(&protocol, state_json, entry, request_path, None)
    }

    #[test]
    fn test_simple_render() {
        let mut files = HashMap::new();
        files.insert(
            "index.html".to_string(),
            "<h1>Hello, {{name}}!</h1>".to_string(),
        );

        let result = render_files_for_test(&files, r#"{"name": "WebUI"}"#, "index.html", "/");
        assert_eq!(result.unwrap(), "<h1>Hello, WebUI!</h1>");
    }

    #[test]
    fn test_missing_entry_file() {
        let files = HashMap::new();
        let result = render_files_for_test(&files, "{}", "index.html", "/");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"), "Unexpected error: {}", err);
    }

    #[test]
    fn test_build_protocol_surfaces_invalid_w_ref() {
        let mut files = HashMap::new();
        files.insert(
            "index.html".to_string(),
            "<my-card>Hi</my-card>".to_string(),
        );
        files.insert(
            "my-card.html".to_string(),
            r#"<div><input w-ref="myInput" /></div>"#.to_string(),
        );

        let result = parser::build_protocol_inner(&files, "index.html", &[]);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("invalid w-ref binding"),
            "Unexpected error: {err}"
        );
        assert!(
            err.contains("component <my-card> · element <input>"),
            "Unexpected error: {err}"
        );
    }

    #[test]
    fn test_render_files_surfaces_invalid_event_handler() {
        let mut files = HashMap::new();
        files.insert("index.html".to_string(), "<my-btn>x</my-btn>".to_string());
        files.insert(
            "my-btn.html".to_string(),
            r#"<div><button @click="e.preventDefault()">x</button></div>"#.to_string(),
        );

        let result = render_files_for_test(&files, "{}", "index.html", "/");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("invalid @click handler"),
            "Unexpected error: {err}"
        );
        assert!(
            err.contains("component <my-btn> · element <button>"),
            "Unexpected error: {err}"
        );
    }

    #[test]
    fn test_with_component() {
        let mut files = HashMap::new();
        files.insert(
            "index.html".to_string(),
            "<my-card>World</my-card>".to_string(),
        );
        files.insert(
            "my-card.html".to_string(),
            "<div class=\"card\"><slot></slot></div>".to_string(),
        );

        let result = render_files_for_test(&files, "{}", "index.html", "/");
        assert!(result.is_ok(), "Render failed: {:?}", result);
        let html = result.as_deref().unwrap_or("");
        assert!(html.contains("card"), "Expected card class in: {}", html);
    }

    #[test]
    fn test_with_for_loop() {
        let mut files = HashMap::new();
        files.insert(
            "index.html".to_string(),
            "<for each=\"item in items\">{{item.name}}, </for>".to_string(),
        );

        let state = r#"{"items": [{"name": "A"}, {"name": "B"}]}"#;
        let result = render_files_for_test(&files, state, "index.html", "/");
        assert!(result.is_ok(), "Render failed: {:?}", result);
        let html = result.as_deref().unwrap_or("");
        assert!(html.contains("A"), "Expected 'A' in: {}", html);
        assert!(html.contains("B"), "Expected 'B' in: {}", html);
    }

    #[test]
    fn test_with_if_condition() {
        let mut files = HashMap::new();
        files.insert(
            "index.html".to_string(),
            "<if condition=\"show\">Visible</if>".to_string(),
        );

        let result_true = render_files_for_test(&files, r#"{"show": true}"#, "index.html", "/");
        assert_eq!(result_true.unwrap(), "Visible");

        let result_false = render_files_for_test(&files, r#"{"show": false}"#, "index.html", "/");
        assert_eq!(result_false.unwrap(), "");
    }

    #[test]
    fn test_invalid_state_json() {
        let mut files = HashMap::new();
        files.insert("index.html".to_string(), "<p>Hi</p>".to_string());

        let result = render_files_for_test(&files, "not json", "index.html", "/");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("State JSON error"),
            "Unexpected error: {}",
            err
        );
    }

    #[test]
    fn test_component_with_css() {
        let mut files = HashMap::new();
        files.insert(
            "index.html".to_string(),
            "<my-card>Content</my-card>".to_string(),
        );
        files.insert(
            "my-card.html".to_string(),
            "<p><slot></slot></p>".to_string(),
        );
        files.insert("my-card.css".to_string(), "p { color: red; }".to_string());

        let result = render_files_for_test(&files, "{}", "index.html", "/");
        assert!(result.is_ok(), "Render failed: {:?}", result);
        let html = result.as_deref().unwrap_or("");
        assert!(
            html.contains("<style>p { color: red; }</style>"),
            "Expected inline <style> tag in: {}",
            html
        );
        assert!(
            !html.contains("<link"),
            "Should not have external <link> tag in: {}",
            html
        );
    }

    #[test]
    fn test_raw_signal() {
        let mut files = HashMap::new();
        files.insert(
            "index.html".to_string(),
            "<div>{{{raw_html}}}</div>".to_string(),
        );

        let result =
            render_files_for_test(&files, r#"{"raw_html": "<b>bold</b>"}"#, "index.html", "/");
        assert!(result.is_ok(), "Render failed: {:?}", result);
        let html = result.as_deref().unwrap_or("");
        assert!(
            html.contains("<b>bold</b>"),
            "Expected raw HTML in: {}",
            html
        );
    }

    #[test]
    fn test_static_html_passthrough() {
        let mut files = HashMap::new();
        files.insert(
            "index.html".to_string(),
            "<h1>Static</h1><p>Content</p>".to_string(),
        );

        let result = render_files_for_test(&files, "{}", "index.html", "/");
        assert_eq!(result.unwrap(), "<h1>Static</h1><p>Content</p>");
    }

    #[test]
    fn test_protocol_tokens_empty() {
        let protocol = WebUIProtocol::new(HashMap::new());
        let bytes = protocol.to_protobuf().unwrap();
        let decoded = WebUIProtocol::from_protobuf(&bytes).unwrap();
        assert!(decoded.tokens.is_empty());
    }

    #[test]
    fn test_protocol_tokens_roundtrip() {
        let tokens = vec![
            "colorBrandBackground".to_string(),
            "fontSizeBase300".to_string(),
        ];
        let protocol = WebUIProtocol::with_tokens(HashMap::new(), tokens.clone());
        let bytes = protocol.to_protobuf().unwrap();
        let decoded = WebUIProtocol::from_protobuf(&bytes).unwrap();
        assert_eq!(decoded.tokens, tokens);
    }

    #[test]
    fn test_protocol_tokens_preserves_order() {
        let tokens = vec!["zeta".to_string(), "alpha".to_string(), "zeta".to_string()];
        let protocol = WebUIProtocol::with_tokens(HashMap::new(), tokens.clone());
        let bytes = protocol.to_protobuf().unwrap();
        let decoded = WebUIProtocol::from_protobuf(&bytes).unwrap();
        assert_eq!(decoded.tokens, tokens);
    }
}
