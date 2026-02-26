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

// ── Feature 3: Handlebars at beginning/end of text ──────────────────

#[test]
fn test_handlebars_at_beginning() {
    // Port of: 'should process handlebars from text at beginning'
    let (fragments, _) = parse_and_get_fragments("{{first}}");
    assert_fragments!(fragments, [signal("first"),]);
}

#[test]
fn test_handlebars_at_beginning_and_raw() {
    // Port of: 'should process handlebars from text at beginning and raw'
    let (fragments, _) = parse_and_get_fragments("{{first}}test");
    assert_fragments!(fragments, [signal("first"), raw("test"),]);
}

#[test]
fn test_handlebars_raw_and_end() {
    // Port of: 'should process handlebars from text at raw and end'
    let (fragments, _) = parse_and_get_fragments("test{{first}}");
    assert_fragments!(fragments, [raw("test"), signal("first"),]);
}

// ── Feature 4: Handlebars edge cases ────────────────────────────────

#[test]
fn test_handlebars_invalid_triple_open() {
    // Port of: 'should not process handlebars when invalid'
    let (fragments, _) = parse_and_get_fragments("{{{invalid}}");
    assert_fragments!(fragments, [raw("{{{invalid}}"),]);
}

#[test]
fn test_handlebars_four_open_braces() {
    // Port of: 'should not process handlebars when invalid since triple exists'
    let (fragments, _) = parse_and_get_fragments("{{{{invalid}}");
    assert_fragments!(fragments, [raw("{{{{invalid}}"),]);
}

#[test]
fn test_handlebars_five_open_with_valid_double() {
    // Port of: 'should not process handlebars when invalid but with valid triple'
    let (fragments, _) = parse_and_get_fragments("{{{{{invalid}}");
    assert_fragments!(fragments, [raw("{{{"), signal("invalid"),]);
}
