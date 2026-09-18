// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use crate::plugin::webui::WebUIParserPlugin;
use crate::*;

pub(crate) const RECURSIVE_TREE: &str = concat!(
    "<fragment name=\"tree-items\"><ul><for each=\"{{child in items}}\">",
    "<li><span>{{child.name}}</span><if condition=\"{{child.children.length}}\">",
    "<render fragment=\"tree-items\" scope=\"{{child.children}}\" as=\"items\"></render>",
    "</if></li></for></ul></fragment>",
    "<render fragment=\"tree-items\" scope=\"{{items}}\" as=\"items\"></render>",
);

#[test]
fn whole_braced_directives_match_bare_recursive_protocol() {
    for webui in [false, true] {
        let bare = RECURSIVE_TREE
            .replace("each=\"{{child in items}}\"", "each=\"child in items\"")
            .replace(
                "condition=\"{{child.children.length}}\"",
                "condition=\"child.children.length\"",
            );
        let mut records = Vec::new();
        for source in [bare.as_str(), RECURSIVE_TREE] {
            let mut parser = if webui {
                HtmlParser::with_plugin(Box::new(WebUIParserPlugin::new()))
            } else {
                HtmlParser::new()
            };
            parser.parse("index.html", source).expect("recursive tree");
            records.push(parser.into_fragment_records());
        }
        assert_eq!(records[0], records[1]);
        let protocol = webui_protocol::WebUIProtocol::new(records.remove(0));
        webui_protocol::WebUIProtocol::from_protobuf(&protocol.to_protobuf().expect("encode"))
            .expect("normalized references survive wire validation");
    }
}

#[test]
fn whole_braced_ordinary_directives_match_bare_protocol() {
    let bare = concat!(
        "<for each=\"child in items\"><if condition=\"child.active && ready\">",
        "{{child.name}}</if></for>",
    );
    let wrapped = concat!(
        "<for each=\" \t{{ child in items }} \"><if condition=\" {{ child.active && ready }} \">",
        "{{child.name}}</if></for>",
    );
    let mut ordinary = HtmlParser::new();
    ordinary.parse("index.html", bare).expect("bare directives");
    let mut normalized = HtmlParser::new();
    normalized
        .parse("index.html", wrapped)
        .expect("whole bindings");
    assert_eq!(
        ordinary.into_fragment_records(),
        normalized.into_fragment_records()
    );
}

#[test]
fn malformed_control_wrappers_remain_actionable_errors() {
    for (element, attribute, expression, code) in [
        (
            "for",
            "each",
            "{{{child in items}}}",
            codes::INVALID_FOR_EACH,
        ),
        ("for", "each", "{{child in items}", codes::INVALID_FOR_EACH),
        (
            "for",
            "each",
            "{{child in items}}tail",
            codes::INVALID_FOR_EACH,
        ),
        (
            "for",
            "each",
            "{{child in items}} {{other}}",
            codes::INVALID_FOR_EACH,
        ),
        ("for", "each", "{{child}}", codes::INVALID_FOR_EACH),
        (
            "for",
            "each",
            "{{child in items[0]}}",
            codes::INVALID_FOR_IDENTIFIER,
        ),
        (
            "if",
            "condition",
            "{{{ready}}}",
            codes::INVALID_IF_CONDITION,
        ),
        ("if", "condition", "{{ready}", codes::INVALID_IF_CONDITION),
        (
            "if",
            "condition",
            "{{ready}}tail",
            codes::INVALID_IF_CONDITION,
        ),
        (
            "if",
            "condition",
            "{{ready}} && {{other}}",
            codes::INVALID_IF_CONDITION,
        ),
        ("if", "condition", "{{ }}", codes::INVALID_IF_CONDITION),
    ] {
        let source =
            format!("<body>\n<{element} {attribute}=\"{expression}\">content</{element}>\n</body>");
        let error = HtmlParser::new()
            .parse("index.html", &source)
            .expect_err(&source);
        let ParserError::Template(diagnostic) = error else {
            panic!("expected template diagnostic");
        };
        assert_eq!(diagnostic.error_code(), Some(code), "{expression}");
        assert_eq!(diagnostic.component_name(), Some("index.html"));
        assert_eq!(diagnostic.position_line_column(), Some((2, 1)));
        assert!(!diagnostic.to_string().contains('\x1b'));
    }
}
