// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use crate::named_fragments::record_id;
use crate::*;

#[test]
fn ordinary_component_usage_preserves_exact_remaining_size_hints() {
    let mut parser = HtmlParser::new();
    let mut source = String::from("<body>");
    for index in 0..9 {
        let tag = format!("x-item-{index}");
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(&tag, "<b>item</b>", None, false))
            .expect("register component");
        source.push_str(&format!("<{tag}></{tag}>"));
    }
    source.push_str("</body>");
    parser.parse("index.html", &source).expect("ordinary entry");
    let mut usage = parser.component_shadow_dom_usage();
    for remaining in (1..=9).rev() {
        assert_eq!(usage.size_hint(), (remaining, Some(remaining)));
        assert!(usage.next().is_some());
    }
    assert_eq!(usage.size_hint(), (0, Some(0)));
    assert_eq!(usage.next(), None);
}

#[test]
fn named_component_usage_filters_incomplete_records_after_parse_errors() {
    let mut parser = HtmlParser::new();
    parser
        .component_registry_mut()
        .register_component(ComponentRegistration::new(
            "x-broken",
            "<if>missing condition</if>",
            None,
            false,
        ))
        .expect("register component");
    parser
        .parse(
            "index.html",
            "<body><fragment name=\"unused\"><x-broken></x-broken></fragment></body>",
        )
        .expect_err("invalid declaration body");
    assert!(parser.component_dom_analyses.contains_key("x-broken"));
    assert!(!parser.has_fragment("x-broken"));
    let mut usage = parser.component_shadow_dom_usage();
    assert_eq!(usage.size_hint(), (0, Some(1)));
    assert_eq!(usage.next(), None);
    assert_eq!(usage.size_hint(), (0, Some(0)));
}

#[test]
fn scriptless_scopes_do_not_track_records_from_another_owner() {
    for source in [
        "<if condition=\"ready\">content</if>",
        "<for each=\"item in items\">content</for>",
    ] {
        let mut parser = HtmlParser::new();
        parser
            .parse(
                "script.html",
                "<body><script type=\"module\" src=\"/entry.js\"></script></body>",
            )
            .expect("script owner");
        assert!(parser.module_entry_sites.is_some());
        let Event::Element(element) = Walker::new(source).next().expect("directive") else {
            panic!("expected directive element");
        };
        let mut fragments = Vec::new();
        let mut ops = Vec::new();
        if element.name() == "if" {
            parser
                .enter_if_directive(&element, &mut fragments, 0, &mut ops)
                .expect("if");
        } else {
            parser
                .enter_for_directive(&element, &mut fragments, 0, &mut ops)
                .expect("for");
        }
        assert!(parser.current_record_id.is_empty(), "{source}");
        assert_eq!(ops.len(), 2, "{source}");
        assert!(!ops.iter().any(|op| matches!(op, ParseOp::RestoreRecord(_))));
    }
}

#[test]
fn tracked_scopes_restore_records_before_completing_their_body() {
    for source in [
        "<if condition=\"ready\">content</if>",
        "<for each=\"item in items\">content</for>",
    ] {
        let mut parser = HtmlParser::new();
        parser.track_owner_records = true;
        parser.current_record_id = "parent-record".to_string();
        let Event::Element(element) = Walker::new(source).next().expect("directive") else {
            panic!("expected directive element");
        };
        let mut fragments = Vec::new();
        let mut ops = Vec::new();
        if element.name() == "if" {
            parser
                .enter_if_directive(&element, &mut fragments, 0, &mut ops)
                .expect("if");
        } else {
            parser
                .enter_for_directive(&element, &mut fragments, 0, &mut ops)
                .expect("for");
        }
        assert!(matches!(ops.pop(), Some(ParseOp::Parse { .. })));
        assert!(
            matches!(ops.pop(), Some(ParseOp::RestoreRecord(previous)) if previous == "parent-record")
        );
        assert!(matches!(
            ops.pop(),
            Some(ParseOp::CompleteIf { .. } | ParseOp::CompleteFor { .. })
        ));
        assert!(ops.is_empty());
    }
}

#[test]
fn mixed_owner_scopes_retain_module_order_after_late_fragment_registration() {
    let mut parser = HtmlParser::with_options(DomStrategy::Light);
    for (tag, source) in [
        (
            "x-shell",
            r#"<if condition="ready"><for each="item in items"><x-leaf></x-leaf></for></if>"#,
        ),
        (
            "x-leaf",
            concat!(
                r#"<if condition="ready"><for each="row in rows"><script type="module" src="/leaf-inner.js"></script></for>"#,
                r#"<for each="row in rows"><!-- empty --></for><script type="module" src="/leaf-after.js"></script></if>"#,
                r#"<script type="module" src="/leaf-tail.js"></script>"#,
            ),
        ),
    ] {
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(tag, source, None, false))
            .expect("register ordinary owner");
    }
    parser
        .parse("first.html", "<body><x-shell></x-shell></body>")
        .expect("initial ordinary graph");
    assert_eq!(
        parser.module_entry_srcs(),
        ["/leaf-inner.js", "/leaf-after.js", "/leaf-tail.js"]
    );
    parser
        .component_registry_mut()
        .register_component(ComponentRegistration::new(
            "x-named",
            concat!(
                r#"<fragment name="tree"><if condition="ready"><script type="module" src="/named.js"></script>"#,
                r#"<if condition="again"><render fragment="tree"/></if></if></fragment><render fragment="tree"/>"#,
            ),
            None,
            false,
        ))
        .expect("register recursive fragment owner");
    parser
        .parse(
            "second.html",
            concat!(
                r#"<body><script type="module" src="/before.js"></script>"#,
                "<x-shell></x-shell><x-named></x-named>",
                r#"<script type="module" src="/after.js"></script></body>"#,
            ),
        )
        .expect("later named graph");
    assert_eq!(
        parser.module_entry_srcs(),
        [
            "/before.js",
            "/leaf-inner.js",
            "/leaf-after.js",
            "/leaf-tail.js",
            "/named.js",
            "/after.js",
        ]
    );
    assert!(parser.current_record_id.is_empty());
    assert!(!parser.track_owner_records);
    let sites = parser
        .module_entry_sites
        .as_ref()
        .expect("module provenance");
    assert!(!sites.contains_key("x-shell"));
    assert!(sites.contains_key("x-leaf"));
    assert!(parser.has_fragment(&record_id("x-named", "tree")));
}

#[test]
fn owner_script_tracking_covers_scanner_accepted_tag_whitespace() {
    for byte in (0u8..=127).filter(|byte| byte.is_ascii_whitespace()) {
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        let source = format!(
            "<if condition=\"ready\"><{}script type=\"module\" src=\"/child.js\"></script></if>",
            char::from(byte)
        );
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new("x-child", &source, None, false))
            .expect("register script owner");
        parser
            .parse(
                "first.html",
                r#"<body><script type="module" src="/entry.js"></script><x-child></x-child></body>"#,
            )
            .expect("ordinary script graph");
        let sites = parser.module_entry_sites.as_ref().expect("module sites");
        assert!(sites.contains_key("if-1"));
        assert!(!sites.contains_key("x-child"));
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "x-named",
                "<fragment name=\"unused\"/>",
                None,
                false,
            ))
            .expect("register later named owner");
        parser
            .parse(
                "second.html",
                "<body><x-child></x-child><x-named></x-named></body>",
            )
            .expect("reuse module provenance");
        assert_eq!(parser.module_entry_srcs(), ["/child.js"]);
    }
}

#[test]
fn scriptless_named_bodies_restore_css_ownership_after_nested_scopes() {
    let mut parser = HtmlParser::new();
    parser
        .parse(
            "index.html",
            concat!(
                r#"<body><fragment name="body"><if condition="ready"><for each="item in items" template="local-loop">"#,
                "<style>:root{--nested:red}</style></for></if>",
                r#"<style>.body{color:var(--nested);--shared:red}</style><render fragment="leaf"/></fragment>"#,
                r#"<fragment name="leaf"><style>.leaf{color:var(--shared)}</style></fragment><render fragment="body"/></body>"#,
            ),
        )
        .expect("scriptless declaration graph");
    assert_eq!(parser.take_tokens(), ["nested"]);
    assert_eq!(
        parser.fragment_css_tokens["local-loop"].definitions,
        ["nested"]
    );
    assert_eq!(
        parser.fragment_css_tokens[&record_id("index.html", "body")].definitions,
        ["shared"]
    );
    assert!(parser.current_record_id.is_empty());
    assert!(!parser.track_owner_records);
}

#[test]
fn failed_nested_owner_restores_record_tracking_before_reparse() {
    let mut parser = HtmlParser::new();
    parser
        .component_registry_mut()
        .register_component(ComponentRegistration::new(
            "x-broken",
            "<if condition=\"ready\"><for each=\"invalid\">content</for></if>",
            None,
            false,
        ))
        .expect("register scriptless owner");
    parser
        .parse(
            "index.html",
            concat!(
                r#"<body><fragment name="body"><script type="module" src="/unused.js"></script>"#,
                r#"<x-broken></x-broken></fragment><render fragment="body"/></body>"#,
            ),
        )
        .expect_err("invalid nested repeat");
    assert!(parser.current_record_id.is_empty());
    assert!(!parser.track_owner_records);
    parser
        .parse(
            "index.html",
            "<body><if condition=\"ready\">ordinary</if></body>",
        )
        .expect("reparse after failure");
    assert!(parser.module_entry_srcs().is_empty());
    assert!(parser.current_record_id.is_empty());
    assert!(!parser.track_owner_records);
}
