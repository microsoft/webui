use webui_test_utils::*;

use crate::*;

// ── Attribute fragment tests ─────────────────────────────────────────

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

#[test]
fn test_attribute_handlebars_in_href() {
    // Port of: 'should process handlebars from attributes as signals'
    let (fragments, _) = parse_and_get_fragments(r#"<a href="{{url}}">{{name}}</a>"#);
    assert_fragments!(
        fragments,
        [
            raw("<a"),
            attr("href", "url"),
            raw(">"),
            signal("name"),
            raw("</a>"),
        ]
    );
}

#[test]
fn test_attribute_boolean_with_handlebars() {
    // Port of: 'should process boolean attribute with handlebars expression'
    let (fragments, _) = parse_and_get_fragments("<button ?disabled={{isDisabled}}>Click</button>");
    assert_fragments!(
        fragments,
        [
            raw("<button"),
            bool_attr("disabled", "isDisabled"),
            raw(">Click</button>"),
        ]
    );
}

#[test]
fn test_attribute_multiple_boolean() {
    // Port of: 'should process multiple boolean attributes'
    // <input ?checked={{isChecked}} ?disabled={{isDisabled}} />
    let (fragments, _) =
        parse_and_get_fragments("<input ?checked={{isChecked}} ?disabled={{isDisabled}} />");

    assert_fragments!(
        fragments,
        [
            raw("<input"),
            bool_attr("checked", "isChecked"),
            bool_attr("disabled", "isDisabled"),
            raw("/>"),
        ]
    );
}

#[test]
fn test_attribute_boolean_and_regular_together() {
    // Port of: 'should process a boolean attribute and a regular attribute together'
    // <input ?checked="{{isChecked}}" type="checkbox">Hi</input>
    let (fragments, _) =
        parse_and_get_fragments(r#"<input ?checked="{{isChecked}}" type="checkbox">Hi</input>"#);

    assert_fragments!(
        fragments,
        [
            raw("<input"),
            bool_attr("checked", "isChecked"),
            raw(" type=\"checkbox\">Hi</input>"),
        ]
    );
}

#[test]
fn test_attribute_boolean_sandwiched() {
    // Port of: 'should process a boolean attribute sandwiched between regular attributes'
    // <input version={{edition}} ?checked="{{isChecked}}" type="checkbox">Hi</input>
    let (fragments, _) = parse_and_get_fragments(
        r#"<input version={{edition}} ?checked="{{isChecked}}" type="checkbox">Hi</input>"#,
    );

    assert_fragments!(
        fragments,
        [
            raw("<input"),
            attr("version", "edition"),
            bool_attr("checked", "isChecked"),
            raw(" type=\"checkbox\">Hi</input>"),
        ]
    );
}

#[test]
fn test_attribute_boolean_ending() {
    // Port of: 'should process html ending with boolean attribute correctly'
    // <input version={{edition}} ?checked="{{isChecked}}">Hi</input>
    let (fragments, _) = parse_and_get_fragments(
        r#"<input version={{edition}} ?checked="{{isChecked}}">Hi</input>"#,
    );

    assert_fragments!(
        fragments,
        [
            raw("<input"),
            attr("version", "edition"),
            bool_attr("checked", "isChecked"),
            raw(">Hi</input>"),
        ]
    );
}

#[test]
fn test_attribute_boolean_dotted_path() {
    // Port of: 'should process boolean attribute with dotted path'
    // <div ?checked={{layout.isPinned}}>Content</div>
    let (fragments, _) = parse_and_get_fragments("<div ?checked={{layout.isPinned}}>Content</div>");

    assert_fragments!(
        fragments,
        [
            raw("<div"),
            bool_attr("checked", "layout.isPinned"),
            raw(">Content</div>"),
        ]
    );
}

#[test]
fn test_attribute_colon_prefixed_complex() {
    // Port of: 'should process colon-prefixed attribute with handlebars'
    // <my-component :config="{{settings}}"></my-component>
    let (fragments, _) =
        parse_and_get_fragments(r#"<my-component :config="{{settings}}"></my-component>"#);

    assert_fragments!(
        fragments,
        [
            raw("<my-component"),
            attr_complex(":config", "settings"),
            raw("></my-component>"),
        ]
    );
}

#[test]
fn test_attribute_multiple_colon_prefixed() {
    // Port of: 'should process multiple colon-prefixed complex attributes'
    // <my-component :prop1="{{val1}}" :prop2="{{val2}}"></my-component>
    let (fragments, _) = parse_and_get_fragments(
        r#"<my-component :prop1="{{val1}}" :prop2="{{val2}}"></my-component>"#,
    );

    assert_fragments!(
        fragments,
        [
            raw("<my-component"),
            attr_complex(":prop1", "val1"),
            attr_complex(":prop2", "val2"),
            raw("></my-component>"),
        ]
    );
}

#[test]
fn test_attribute_mixed_normal_boolean_colon() {
    // Port of: 'should process mixed normal, boolean, and colon-prefixed attributes'
    // <my-component id="comp" :config="{{settings}}" ?enabled="{{isEnabled}}"></my-component>
    let (fragments, _) = parse_and_get_fragments(
        r#"<my-component id="comp" :config="{{settings}}" ?enabled="{{isEnabled}}"></my-component>"#,
    );

    assert_fragments!(
        fragments,
        [
            raw("<my-component id=\"comp\""),
            attr_complex(":config", "settings"),
            bool_attr("enabled", "isEnabled"),
            raw("></my-component>"),
        ]
    );
}

#[test]
fn test_attribute_reject_boolean_without_handlebars() {
    // Port of: 'should reject boolean attribute without handlebars'
    // <input ?checked="name"></input>
    let (fragments, _) = parse_and_get_fragments(r#"<input ?checked="name"></input>"#);

    // Boolean attribute is silently dropped
    assert_fragments!(fragments, [raw("<input></input>"),]);
}

#[test]
fn test_attribute_reject_boolean_with_partial_handlebars() {
    // Port of: 'should reject boolean attribute with partial handlebars'
    // <input ?checked="Hello {{name}}"></input>
    let (fragments, _) = parse_and_get_fragments(r#"<input ?checked="Hello {{name}}"></input>"#);

    // Boolean attribute is silently dropped
    assert_fragments!(fragments, [raw("<input></input>"),]);
}

#[test]
fn test_attribute_reject_boolean_with_plain_value() {
    // Port of: 'should reject boolean attribute with plain value'
    // <button ?disabled="true">Click</button>
    let (fragments, _) = parse_and_get_fragments(r#"<button ?disabled="true">Click</button>"#);

    // Boolean attribute is silently dropped
    assert_fragments!(fragments, [raw("<button>Click</button>"),]);
}

#[test]
fn test_attribute_mixed_static_dynamic() {
    // Port of: 'should process mixed attributes correctly'
    // <input value="hello {{world}}">Hi</input>
    let (fragments, records) =
        parse_and_get_fragments(r#"<input value="hello {{world}}">Hi</input>"#);

    assert_fragments!(
        fragments,
        [
            raw("<input"),
            attr_template("value", "attr-1"),
            raw(">Hi</input>"),
        ]
    );

    // Verify the template sub-stream
    assert_stream!(records, "attr-1", [raw("hello "), signal("world"),]);
}
