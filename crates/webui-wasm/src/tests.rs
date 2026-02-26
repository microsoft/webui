use super::*;

#[test]
fn test_simple_render() {
    let mut files = HashMap::new();
    files.insert(
        "index.html".to_string(),
        "<h1>Hello, {{name}}!</h1>".to_string(),
    );

    let result = build_and_render_inner(&files, r#"{"name": "WebUI"}"#, "index.html");
    assert!(result.is_ok(), "Render failed: {:?}", result);
    assert_eq!(result.as_deref(), Ok("<h1>Hello, WebUI!</h1>"));
}

#[test]
fn test_missing_entry_file() {
    let files = HashMap::new();
    let result = build_and_render_inner(&files, "{}", "index.html");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("not found"), "Unexpected error: {}", err);
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

    let result = build_and_render_inner(&files, "{}", "index.html");
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
    let result = build_and_render_inner(&files, state, "index.html");
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

    let result_true = build_and_render_inner(&files, r#"{"show": true}"#, "index.html");
    assert_eq!(result_true.as_deref(), Ok("Visible"));

    let result_false = build_and_render_inner(&files, r#"{"show": false}"#, "index.html");
    assert_eq!(result_false.as_deref(), Ok(""));
}

#[test]
fn test_invalid_state_json() {
    let mut files = HashMap::new();
    files.insert("index.html".to_string(), "<p>Hi</p>".to_string());

    let result = build_and_render_inner(&files, "not json", "index.html");
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

    let result = build_and_render_inner(&files, "{}", "index.html");
    assert!(result.is_ok(), "Render failed: {:?}", result);
    let html = result.as_deref().unwrap_or("");
    // WASM uses CssStrategy::Inline, so CSS should be in <style> tags, not <link>
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

    let result = build_and_render_inner(&files, r#"{"raw_html": "<b>bold</b>"}"#, "index.html");
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

    let result = build_and_render_inner(&files, "{}", "index.html");
    assert_eq!(result.as_deref(), Ok("<h1>Static</h1><p>Content</p>"));
}
