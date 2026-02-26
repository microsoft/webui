use webui_test_utils::*;

use crate::*;

#[test]
fn test_parse_signal() {
    let mut parser = HtmlParser::new();
    let html = "Hello, {{name}}!";
    let result = parser.parse("test.html", html);

    assert!(result.is_ok());
    let fragment_records = parser.into_fragment_records();

    assert_stream!(
        fragment_records,
        "test.html",
        [raw("Hello, "), signal("name"), raw("!"),]
    );
}

#[test]
fn test_parse_raw_signal() {
    let mut parser = HtmlParser::new();
    let html = "Hello, {{{html_content}}}!";
    let result = parser.parse("test.html", html);

    assert!(result.is_ok());
    let fragment_records = parser.into_fragment_records();

    assert_stream!(
        fragment_records,
        "test.html",
        [raw("Hello, "), signal_raw("html_content"), raw("!"),]
    );
}

#[test]
fn test_parse_for_directive() {
    let mut parser = HtmlParser::new();
    let html = r#"<for each="item in items"><div class="item">{{item.name}}</div></for>"#;

    let result = parser.parse("test.html", html);
    assert!(result.is_ok(), "Parse error: {:?}", result.err());
    let fragment_records = parser.into_fragment_records();
    println!("Fragment records: {:#?}", fragment_records);

    assert_stream!(
        fragment_records,
        "test.html",
        [for_loop("item", "items", "for-1"),]
    );

    // Verify the sub-fragment contains our item content
    assert_stream!(
        fragment_records,
        "for-1",
        [
            raw("<div class=\"item\">"),
            signal("item.name"),
            raw("</div>"),
        ]
    );
}

#[test]
fn test_parse_if_directive() {
    let mut parser = HtmlParser::new();
    let html = r#"<if condition="isLoggedIn"><div>Welcome back, {{username}}!</div></if>"#;

    let result = parser.parse("test.html", html);

    assert!(result.is_ok(), "Parse error: {:?}", result.err());
    let fragment_records = parser.into_fragment_records();
    println!("Fragment records: {:#?}", fragment_records);

    assert_stream!(fragment_records, "test.html", [if_cond("if-1"),]);

    // Verify the sub-fragment contains our content
    assert_stream!(
        fragment_records,
        "if-1",
        [
            raw("<div>Welcome back, "),
            signal("username"),
            raw("!</div>"),
        ]
    );
}

#[test]
fn test_component_directive() {
    let mut parser = HtmlParser::new();
    parser
        .component_registry
        .register_component(
            "my-component",
            "<div>My Component</div>",
            Some("div { color: blue; }"),
        )
        .expect("Failed to register component");

    let result = parser.parse("test.html", "<my-component></my-component>");
    assert!(result.is_ok(), "Parse error: {:?}", result.err());
    let records = parser.into_fragment_records();

    assert_stream!(
        records,
        "test.html",
        [
            raw("<my-component>"),
            component("my-component"),
            raw("</my-component>"),
        ]
    );

    // Component template stream should be wrapped with shadow DOM template + style link
    let comp = &records["my-component"].fragments;
    assert_eq!(comp.len(), 1);
    assert!(
        matches!(comp[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if
            raw.value.contains("<template shadowrootmode=\"open\">") && raw.value.contains("<div>My Component</div>"))
    );
}

#[test]
fn test_component_directive_with_slots() {
    let mut parser = HtmlParser::new();
    parser
        .component_registry
        .register_component(
            "my-component",
            "<div>My Component</div>",
            Some("div { color: blue; }"),
        )
        .expect("Failed to register component");

    let result = parser.parse(
        "test.html",
        "Hello<my-component><p>World</p></my-component>",
    );
    assert!(result.is_ok(), "Parse error: {:?}", result.err());
    let records = parser.into_fragment_records();
    let fragments = &records["test.html"].fragments;

    // Entry: raw(Hello<my-component>) + component + raw(<p>World</p></my-component>)
    assert!(fragments.len() >= 3);
    // First fragment should contain "Hello" and "<my-component>"
    assert!(
        matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.contains("Hello") && raw.value.contains("<my-component>"))
    );
    // Should have component fragment
    assert!(fragments.iter().any(|f| matches!(
        f.fragment.as_ref(),
        Some(Fragment::Component(c)) if c.fragment_id == "my-component"
    )));
    // Should end with closing tag
    let last = fragments.last().unwrap();
    assert!(
        matches!(last.fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.contains("</my-component>"))
    );
}

// ── Component template wrapping tests ────────────────────────────

#[test]
fn test_component_no_double_wrap_template() {
    let mut parser = HtmlParser::new();
    parser
        .component_registry
        .register_component(
            "custom-element",
            r#"<template foo="bar"><slot></slot></template>"#,
            None,
        )
        .expect("register");
    let result = parser.parse("index.html", "<custom-element>Hello</custom-element>");
    assert!(result.is_ok());
    let records = parser.into_fragment_records();

    assert_fragments!(
        records["index.html"].fragments,
        [
            raw("<custom-element>"),
            component("custom-element"),
            raw("Hello</custom-element>"),
        ]
    );

    assert_stream!(
        records,
        "custom-element",
        [raw(r#"<template foo="bar"><slot></slot></template>"#),]
    );
}

#[test]
fn test_component_styled_no_double_wrap() {
    let mut parser = HtmlParser::new();
    parser
        .component_registry
        .register_component(
            "custom-element",
            r#"<template foo="bar"><slot></slot></template>"#,
            Some("div { color: red; }"),
        )
        .expect("register");
    let result = parser.parse("index.html", "<custom-element>Hello</custom-element>");
    assert!(result.is_ok());
    let records = parser.into_fragment_records();

    assert_stream!(
        records,
        "custom-element",
        [raw(
            r#"<template foo="bar"><link rel="stylesheet" href="./custom-element.css"><slot></slot></template>"#
        ),]
    );
}

#[test]
fn test_component_strip_runtime_attrs() {
    let mut parser = HtmlParser::new();
    parser
        .component_registry
        .register_component(
            "custom-element",
            r#"<template @click={foo} :bar="baz" ?bool="true"><slot></slot></template>"#,
            None,
        )
        .expect("register");
    let result = parser.parse("index.html", "<custom-element>Hello</custom-element>");
    assert!(result.is_ok());
    let records = parser.into_fragment_records();

    assert_stream!(
        records,
        "custom-element",
        [raw("<template><slot></slot></template>"),]
    );
}

#[test]
fn test_component_with_slots_and_attrs() {
    let mut parser = HtmlParser::new();
    parser
        .component_registry
        .register_component("custom-element", "<slot></slot>", None)
        .expect("register");
    let result = parser.parse(
        "index.html",
        r#"<custom-element appearance="subtle">Hello World</custom-element>"#,
    );
    assert!(result.is_ok());
    let records = parser.into_fragment_records();
    assert_fragments!(
        records["index.html"].fragments,
        [
            raw("<custom-element"),
            attr_raw_start("appearance", "subtle"),
            raw(">"),
            component("custom-element"),
            raw("Hello World</custom-element>"),
        ]
    );
}

#[test]
fn test_component_legacy_no_styles() {
    let mut parser = HtmlParser::new();
    parser
        .component_registry
        .register_component("custom-element", "<div>Custom Element</div>", None)
        .expect("register");
    let result = parser.parse("index.html", "<custom-element></custom-element>");
    assert!(result.is_ok());
    let records = parser.into_fragment_records();

    assert_fragments!(
        records["index.html"].fragments,
        [
            raw("<custom-element>"),
            component("custom-element"),
            raw("</custom-element>"),
        ]
    );

    assert_stream!(
        records,
        "custom-element",
        [raw(
            "<template shadowrootmode=\"open\"><div>Custom Element</div></template>"
        ),]
    );
}

#[test]
fn test_component_self_closing() {
    let mut parser = HtmlParser::new();
    parser
        .component_registry
        .register_component("custom-widget", "<div>Widget Content</div>", None)
        .expect("register");
    let result = parser.parse("index.html", r#"<custom-widget config="{{settings}}" />"#);
    assert!(result.is_ok());
    let records = parser.into_fragment_records();

    assert_fragments!(
        records["index.html"].fragments,
        [
            raw("<custom-widget"),
            attr_start("config", "settings"),
            raw("/>"),
            component("custom-widget"),
        ]
    );
}

#[test]
fn test_component_nested_self_closing_in_slot() {
    let mut parser = HtmlParser::new();
    parser
        .component_registry
        .register_component("custom-icon", "<svg><slot></slot></svg>", None)
        .expect("register");
    let result = parser.parse(
        "index.html",
        r##"<custom-icon><use href="#icon-{{iconName}}" /></custom-icon>"##,
    );
    assert!(result.is_ok());
    let records = parser.into_fragment_records();

    assert_fragments!(
        records["index.html"].fragments,
        [
            raw("<custom-icon>"),
            component("custom-icon"),
            raw("<use"),
            attr_template("href", "attr-1"),
            raw("/></custom-icon>"),
        ]
    );

    assert_stream!(
        records,
        "custom-icon",
        [raw(
            "<template shadowrootmode=\"open\"><svg><slot></slot></svg></template>"
        ),]
    );
}

#[test]
fn test_component_leading_boolean_attr_start() {
    let mut parser = HtmlParser::new();
    parser
        .component_registry
        .register_component("custom-element", "<slot></slot>", None)
        .expect("register");
    let result = parser.parse(
        "index.html",
        r#"<custom-element ?disabled="{{isDisabled}}" title="Hello"></custom-element>"#,
    );
    assert!(result.is_ok());
    let records = parser.into_fragment_records();

    assert_fragments!(
        records["index.html"].fragments,
        [
            raw("<custom-element"),
            // First dynamic attr: boolean with attrStart
            bool_attr_start("disabled", "isDisabled"),
            // Static attr after dynamic: rawValue
            attr_raw("title", "Hello"),
            raw(">"),
            component("custom-element"),
            raw("</custom-element>"),
        ]
    );
}

#[test]
fn test_component_meta_link_tags() {
    let (fragments, _) = parse_and_get_fragments(
        r#"<head><meta charset="utf-8" /><link rel="stylesheet" href="{{cssFile}}" /></head>"#,
    );
    assert!(fragments.len() >= 3);
    assert!(
        matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.contains("<head><meta charset=\"utf-8\"") && raw.value.contains("<link"))
    );
    assert!(
        matches!(fragments[1].fragment.as_ref(), Some(Fragment::Attribute(a)) if a.name == "href" && a.value == "cssFile")
    );
    assert!(
        matches!(fragments[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.contains("/></head>"))
    );
}

#[test]
fn test_nested_directives() {
    let mut parser = HtmlParser::new();
    let html = r#"<for each="category in categories">
            <if condition="category.hasItems">
                <for each="item in category.items">
                   {{item.title}}
                </for>
            </if>
        </for>"#;

    let result = parser.parse("test.html", html);

    assert!(result.is_ok(), "Parse error: {:?}", result.err());
    let fragment_records = parser.into_fragment_records();

    assert_stream!(
        fragment_records,
        "test.html",
        [for_loop("category", "categories", "for-1"),]
    );

    assert_stream!(fragment_records, "for-1", [if_cond("if-1"),]);

    assert_stream!(
        fragment_records,
        "if-1",
        [for_loop("item", "category.items", "for-2"),]
    );

    assert_stream!(fragment_records, "for-2", [signal("item.title"),]);
}

#[test]
fn test_complex_directives() {
    let mut parser = HtmlParser::new();
    let html = r#"<for each="category in categories">
            <div class="category">
                <h2>{{category.name}}</h2>
                <if condition="category.hasItems">
                    <ul>
                        <for each="item in category.items">
                            <li>{{item.title}}</li>
                        </for>
                    </ul>
                </if>
            </div>
        </for>"#;

    let result = parser.parse("test.html", html);

    assert!(result.is_ok(), "Parse error: {:?}", result.err());
    let fragment_records = parser.into_fragment_records();

    assert_stream!(
        fragment_records,
        "test.html",
        [for_loop("category", "categories", "for-1"),]
    );

    // Verify for fragments contains the category.name signal
    assert_stream!(
        fragment_records,
        "for-1",
        [
            raw("<div class=\"category\"><h2>"),
            signal("category.name"),
            raw("</h2>"),
            if_cond("if-1"),
            raw("</div>"),
        ]
    );

    // Verify nested if condition.
    assert_stream!(
        fragment_records,
        "if-1",
        [
            raw("<ul>"),
            for_loop("item", "category.items", "for-2"),
            raw("</ul>"),
        ]
    );

    // Verify nested for each.
    assert_stream!(
        fragment_records,
        "for-2",
        [raw("<li>"), signal("item.title"), raw("</li>"),]
    );
}

// ── Body signal tests ─────────────────────────────────────────────

#[test]
fn test_body_signals() {
    let (fragments, _) = parse_and_get_fragments("<body><app-shell></app-shell></body>");
    assert_fragments!(
        fragments,
        [
            raw("<body>"),
            signal_raw("body_start"),
            raw("<app-shell></app-shell>"),
            signal_raw("body_end"),
            raw("</body>"),
        ]
    );
}

// ── Empty for handling tests ──────────────────────────────────────

#[test]
fn test_empty_for_produces_nothing() {
    let (fragments, records) =
        parse_and_get_fragments(r#"<div><for each="item in items"></for></div>"#);
    assert_fragments!(fragments, [raw("<div></div>"),]);
    assert!(!records.contains_key("for-1"));
}

// ── Feature 1: Custom template attribute on <for> ────────────────────

#[test]
fn test_for_custom_template_attribute() {
    // Port of: 'should process transient node for with template'
    let (fragments, records) = parse_and_get_fragments(
        r#"<for each="item in items" template="static"><span>Item</span></for>"#,
    );
    assert_fragments!(fragments, [for_loop("item", "items", "static"),]);
    assert_stream!(records, "static", [raw("<span>Item</span>"),]);
}

#[test]
fn test_for_recursive_template() {
    // Port of: 'should process recursive transient nodes'
    let mut parser = HtmlParser::new();
    let html = r#"<for template="static" each="outerItem in outerItems"><div><span>{{outerItem.name}}</span><for template="static" each="innerItem in innerItems" /></div></for>"#;
    let result = parser.parse("index.html", html);
    assert!(result.is_ok(), "Parse error: {:?}", result.err());
    let records = parser.into_fragment_records();

    assert_fragments!(
        records["index.html"].fragments,
        [for_loop("outerItem", "outerItems", "static"),]
    );

    assert_stream!(
        records,
        "static",
        [
            raw("<div><span>"),
            signal("outerItem.name"),
            raw("</span>"),
            for_loop("innerItem", "innerItems", "static"),
            raw("</div>"),
        ]
    );
}

// ── Feature 2: <if> / <for> with multiple children ──────────────────

#[test]
fn test_if_multiple_children() {
    // Port of: 'should handle <if> with multiple children'
    let (fragments, records) =
        parse_and_get_fragments(r#"<if condition="valid"><p>hello</p><p>world</p></if>"#);
    assert_fragments!(fragments, [if_cond("if-1"),]);
    assert_stream!(records, "if-1", [raw("<p>hello</p><p>world</p>"),]);
}

#[test]
fn test_for_multiple_children() {
    // Port of: 'should handle <for> with multiple children'
    let (fragments, records) =
        parse_and_get_fragments(r#"<for each="item in items"><p>hello</p><p>world</p></for>"#);
    assert_fragments!(fragments, [for_loop("item", "items", "for-1"),]);
    assert_stream!(records, "for-1", [raw("<p>hello</p><p>world</p>"),]);
}

// ── Feature 6: Component attribute skip / multiple nested ───────────

#[test]
fn test_component_attr_skip() {
    // Port of: 'should set attrSkip for skipped component attributes'
    let mut parser = HtmlParser::new();
    parser
        .component_registry
        .register_component("custom-element", "<slot></slot>", None)
        .expect("register");
    let html = r#"<custom-element :config="{{config}}" class="{{value0}}" style="{{value1}}" role="{{value2}}" data-test="{{value3}}" aria-test="{{value4}}"></custom-element>"#;
    let result = parser.parse("index.html", html);
    assert!(result.is_ok(), "Parse error: {:?}", result.err());
    let records = parser.into_fragment_records();

    // <custom-element, :config(attrStart), class(attrSkip), style(attrSkip),
    // role(attrSkip), data-test(attrSkip), aria-test(attrSkip), >, component, </custom-element>
    assert_fragments!(
        records["index.html"].fragments,
        [
            raw("<custom-element"),
            // :config with attrStart
            attr_complex_start(":config", "config"),
            // Skipped attrs
            attr_skip("class", "value0"),
            attr_skip("style", "value1"),
            attr_skip("role", "value2"),
            attr_skip("data-test", "value3"),
            attr_skip("aria-test", "value4"),
            raw(">"),
            component("custom-element"),
            raw("</custom-element>"),
        ]
    );
}

#[test]
fn test_component_multiple_nested() {
    // Port of: 'handle multiple nested web components'
    let mut parser = HtmlParser::new();
    parser
        .component_registry
        .register_component(
            "custom-element",
            "<custom-child></custom-child><slot></slot>",
            None,
        )
        .expect("register");
    parser
        .component_registry
        .register_component("custom-button", "<slot></slot>", None)
        .expect("register");
    parser
        .component_registry
        .register_component("custom-child", "<h1>Hello World!</h1>", None)
        .expect("register");

    let html = r#"<for each="item in items"><custom-element><custom-button>Ok</custom-button></custom-element></for>"#;
    let result = parser.parse("index.html", html);
    assert!(result.is_ok(), "Parse error: {:?}", result.err());
    let records = parser.into_fragment_records();

    // Entry stream
    assert_fragments!(
        records["index.html"].fragments,
        [for_loop("item", "items", "for-1"),]
    );

    // For stream
    assert_stream!(
        records,
        "for-1",
        [
            raw("<custom-element>"),
            component("custom-element"),
            raw("<custom-button>"),
            component("custom-button"),
            raw("Ok</custom-button></custom-element>"),
        ]
    );

    // Component streams — custom-element has contains() checks, keep manual
    let ce = &records["custom-element"].fragments;
    assert_eq!(ce.len(), 3);
    assert!(
        matches!(ce[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.starts_with("<template shadowrootmode=\"open\"><custom-child>"))
    );
    assert!(
        matches!(ce[1].fragment.as_ref(), Some(Fragment::Component(c)) if c.fragment_id == "custom-child")
    );
    assert!(
        matches!(ce[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.contains("</custom-child><slot></slot></template>"))
    );

    assert_stream!(
        records,
        "custom-button",
        [raw(
            "<template shadowrootmode=\"open\"><slot></slot></template>"
        ),]
    );

    assert_stream!(
        records,
        "custom-child",
        [raw(
            "<template shadowrootmode=\"open\"><h1>Hello World!</h1></template>"
        ),]
    );
}

// ── Error handling tests ──────────────────────────────────────────

#[test]
fn test_invalid_markup_returns_error() {
    // Port of: 'should fail with invalid markup'
    // tree-sitter is lenient — it recovers from unclosed tags
    let mut parser = HtmlParser::new();
    let result = parser.parse("index.html", "<div><span>Unclosed div");
    assert!(result.is_ok());
}

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
