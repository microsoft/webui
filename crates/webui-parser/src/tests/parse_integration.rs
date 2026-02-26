use crate::*;

/// Helper to parse HTML and return the fragments for the entry stream.
fn parse_and_get_fragments(html: &str) -> (Vec<WebUIFragment>, WebUIFragmentRecords) {
    let mut parser = HtmlParser::new();
    let result = parser.parse("index.html", html);
    assert!(result.is_ok(), "Parse error: {:?}", result.err());
    let records = parser.into_fragment_records();
    let fragments = records
        .get("index.html")
        .expect("Failed to get index.html fragment")
        .fragments
        .clone();
    (fragments, records)
}

// ── Integration tests ─────────────────────────────────────────────

#[test]
fn test_complex_raw_text_full_page() {
    // Port of: 'should process a complex raw text'
    let html = r#"<!DOCTYPE HTML><html dir="auto" lang="en"><head><meta charset="utf-8"><title>Test</title><style>html { margin: 0; }</style></head><body><app-shell></app-shell><script type="module" src="./index.js"></script></body></html>"#;
    let (fragments, _) = parse_and_get_fragments(html);

    // DOCTYPE + head + <body>, body_start, body content, body_end, </body></html>
    assert!(fragments.len() >= 5);
    assert!(
        matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if
            raw.value.contains("<!DOCTYPE HTML>") && raw.value.ends_with("<body>"))
    );
    assert!(
        matches!(fragments[1].fragment.as_ref(), Some(Fragment::Signal(s)) if
            s.value == "body_start" && s.raw)
    );
    assert!(
        matches!(fragments[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if
            raw.value.contains("<app-shell>"))
    );
    assert!(
        matches!(fragments[3].fragment.as_ref(), Some(Fragment::Signal(s)) if
            s.value == "body_end" && s.raw)
    );
    assert!(
        matches!(fragments[4].fragment.as_ref(), Some(Fragment::Raw(raw)) if
            raw.value.contains("</body>") && raw.value.contains("</html>"))
    );
}

#[test]
fn test_css_strategy_external_emits_link_tag() {
    let mut parser = HtmlParser::new();
    parser
        .component_registry_mut()
        .register_component("my-card", "<p><slot></slot></p>", Some("p { color: red; }"))
        .ok();
    parser.parse("index.html", "<my-card>Hello</my-card>").ok();
    let records = parser.into_fragment_records();
    let my_card = &records["my-card"].fragments;
    let raw_text: String = my_card
        .iter()
        .filter_map(|f| match &f.fragment {
            Some(Fragment::Raw(r)) => Some(r.value.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        raw_text.contains(r#"<link rel="stylesheet" href="./my-card.css">"#),
        "Expected external <link> tag in: {}",
        raw_text
    );
}

#[test]
fn test_css_strategy_inline_emits_style_tag() {
    let mut parser = HtmlParser::new();
    parser.set_css_strategy(CssStrategy::Inline);
    parser
        .component_registry_mut()
        .register_component("my-card", "<p><slot></slot></p>", Some("p { color: red; }"))
        .ok();
    parser.parse("index.html", "<my-card>Hello</my-card>").ok();
    let records = parser.into_fragment_records();
    let my_card = &records["my-card"].fragments;
    let raw_text: String = my_card
        .iter()
        .filter_map(|f| match &f.fragment {
            Some(Fragment::Raw(r)) => Some(r.value.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        raw_text.contains("<style>p { color: red; }</style>"),
        "Expected inline <style> tag in: {}",
        raw_text
    );
    assert!(
        !raw_text.contains("<link"),
        "Should not have <link> tag in inline mode: {}",
        raw_text
    );
}
