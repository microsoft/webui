use webui_test_utils::*;

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

// ── Self-closing / void element tests ─────────────────────────────

#[test]
fn test_self_closing_svg_path() {
    let (fragments, _) =
        parse_and_get_fragments(r#"<svg width="19"><path d="foo" fill="currentcolor"/></svg>"#);
    assert_fragments!(
        fragments,
        [raw(
            r#"<svg width="19"><path d="foo" fill="currentcolor"/></svg>"#
        ),]
    );
}

#[test]
fn test_html5_void_elements() {
    let (fragments, _) = parse_and_get_fragments(
        r#"<div><img src="test.jpg" alt="test"><br><hr><input type="text"></div>"#,
    );
    assert_fragments!(
        fragments,
        [raw(
            r#"<div><img src="test.jpg" alt="test"><br><hr><input type="text"></div>"#
        ),]
    );
}

#[test]
fn test_self_closing_with_dynamic_attributes() {
    let (fragments, _) =
        parse_and_get_fragments(r#"<img src="{{imageUrl}}" alt="{{imageAlt}}" />"#);
    assert_fragments!(
        fragments,
        [
            raw("<img"),
            attr("src", "imageUrl"),
            attr("alt", "imageAlt"),
            raw("/>"),
        ]
    );
}

#[test]
fn test_self_closing_with_boolean_attributes() {
    let (fragments, _) = parse_and_get_fragments(
        r#"<input type="checkbox" ?checked="{{isSelected}}" ?disabled="{{isDisabled}}" />"#,
    );
    assert_fragments!(
        fragments,
        [
            raw("<input type=\"checkbox\""),
            bool_attr("checked", "isSelected"),
            bool_attr("disabled", "isDisabled"),
            raw("/>"),
        ]
    );
}

#[test]
fn test_multiple_self_closing_in_sequence() {
    let (fragments, _) = parse_and_get_fragments(r#"<img src="1.jpg" /><br /><img src="2.jpg" />"#);
    assert_fragments!(
        fragments,
        [raw(r#"<img src="1.jpg"/><br/><img src="2.jpg"/>"#),]
    );
}

#[test]
fn test_self_closing_with_mixed_content() {
    let (fragments, _) =
        parse_and_get_fragments(r#"<div>Text before<img src="{{url}}" />Text after</div>"#);
    assert_fragments!(
        fragments,
        [
            raw("<div>Text before<img"),
            attr("src", "url"),
            raw("/>Text after</div>"),
        ]
    );
}

#[test]
fn test_self_closing_svg_elements() {
    let (fragments, _) = parse_and_get_fragments(
        r#"<svg><circle cx="{{x}}" cy="{{y}}" r="5" /><rect width="10" height="10" /></svg>"#,
    );
    assert_fragments!(
        fragments,
        [
            raw("<svg><circle"),
            attr("cx", "x"),
            attr("cy", "y"),
            raw(r#" r="5"/><rect width="10" height="10"/></svg>"#),
        ]
    );
}

#[test]
fn test_self_closing_inside_for_loop() {
    let (fragments, records) =
        parse_and_get_fragments(r#"<for each="item in items"><img src="{{item.url}}" /></for>"#);
    assert_fragments!(fragments, [for_loop("item", "items", "for-1"),]);
    assert_stream!(
        records,
        "for-1",
        [raw("<img"), attr("src", "item.url"), raw("/>"),]
    );
}

#[test]
fn test_self_closing_whitespace_variations() {
    let (fragments, _) =
        parse_and_get_fragments(r#"<img src="test.jpg"/><input type="text" /><br/>"#);
    assert_fragments!(
        fragments,
        [raw(r#"<img src="test.jpg"/><input type="text"/><br/>"#),]
    );
}

#[test]
fn test_deeply_nested_self_closing() {
    let (fragments, _) = parse_and_get_fragments(
        r#"<div><section><article><img src="deep.jpg" /><br /></article></section></div>"#,
    );
    assert_fragments!(
        fragments,
        [raw(
            r#"<div><section><article><img src="deep.jpg"/><br/></article></section></div>"#
        ),]
    );
}

#[test]
fn test_self_closing_vs_empty_regular_tags() {
    let (fragments, _) =
        parse_and_get_fragments(r#"<div></div><img src="test.jpg" /><span></span>"#);
    assert_fragments!(
        fragments,
        [raw(r#"<div></div><img src="test.jpg"/><span></span>"#),]
    );
}

#[test]
fn test_entities_preserved() {
    // Port of: 'should process entities correctly'
    let (fragments, _) = parse_and_get_fragments("<p>Hello&#125;World</p>");
    assert_fragments!(fragments, [raw("<p>Hello&#125;World</p>"),]);
}

// ── Feature 5: DOCTYPE handling ─────────────────────────────────────

#[test]
fn test_doctype_preserved() {
    // DOCTYPE should be preserved as raw content
    let (fragments, _) = parse_and_get_fragments("<!DOCTYPE html><html><head></head></html>");
    assert!(fragments.len() >= 1);
    assert!(
        matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.contains("<!DOCTYPE html>"))
    );
}
