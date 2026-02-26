use super::*;
use webui_parser::HtmlParser;

/// Helper: parse HTML into protobuf bytes for testing.
fn build_protocol(html: &str) -> Vec<u8> {
    let mut parser = HtmlParser::new();
    parser.parse("index.html", html).expect("parse failed");
    let protocol = WebUIProtocol {
        fragments: parser.into_fragment_records(),
    };
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
