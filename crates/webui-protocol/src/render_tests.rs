// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use super::*;

#[test]
fn render_constructor_and_recursive_protocol_round_trip() {
    let call = WebUIFragment::render("body", "tree.children", "items");
    assert!(
        matches!(&call.fragment, Some(web_ui_fragment::Fragment::Render(render))
        if render.fragment_id == "body" && render.scope == "tree.children" && render.alias == "items")
    );
    let protocol = WebUIProtocol::new(HashMap::from([
        (
            "entry".into(),
            FragmentList {
                fragments: vec![call],
                contains_boundary: false,
            },
        ),
        (
            "body".into(),
            FragmentList {
                fragments: vec![WebUIFragment::render("body", "items.children", "items")],
                contains_boundary: false,
            },
        ),
    ]));
    let bytes = protocol.to_protobuf().expect("encode");
    let decoded = WebUIProtocol::from_protobuf(&bytes).expect("decode");
    assert_eq!(decoded, protocol);
}

#[test]
fn parameterless_render_omits_optional_input_strings() {
    let fragment = WebUIFragment::render("body", "", "");
    let bytes = fragment.encode_to_vec();
    let decoded = WebUIFragment::decode(bytes.as_slice()).expect("decode");
    assert_eq!(fragment, decoded);
    assert!(
        matches!(decoded.fragment, Some(web_ui_fragment::Fragment::Render(render))
        if render.scope.is_empty() && render.alias.is_empty())
    );
}

#[test]
fn malformed_render_references_are_rejected_on_load() {
    for (target, scope, alias) in [
        ("missing", "", ""),
        ("body", "data", ""),
        ("body", "", "items"),
        ("body", "data.0", "items"),
        ("body", "{{data}}", "items"),
        ("body", "data", "items.children"),
        ("body", "data", "bad-alias"),
    ] {
        let protocol = WebUIProtocol::new(HashMap::from([
            (
                "entry".into(),
                FragmentList {
                    fragments: vec![WebUIFragment::render(target, scope, alias)],
                    contains_boundary: false,
                },
            ),
            ("body".into(), FragmentList::default()),
        ]));
        let error = WebUIProtocol::from_protobuf(&protocol.to_protobuf().expect("encode"))
            .expect_err("invalid render must fail validation");
        assert!(error.to_string().contains("Invalid render"));
    }
}

#[test]
fn style_closure_follows_named_render_cycles_in_source_order() {
    let mut protocol = WebUIProtocol::new(HashMap::from([
        (
            "entry".into(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::render("a", "", ""),
                    WebUIFragment::component("x-after"),
                ],
                contains_boundary: false,
            },
        ),
        (
            "a".into(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::component("x-first"),
                    WebUIFragment::render("b", "", ""),
                ],
                contains_boundary: false,
            },
        ),
        (
            "b".into(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::component("x-second"),
                    WebUIFragment::render("a", "", ""),
                ],
                contains_boundary: false,
            },
        ),
    ]));
    protocol.set_css_strategy(CssStrategy::Style);
    for tag in ["x-first", "x-second", "x-after"] {
        protocol
            .fragments
            .insert(tag.into(), FragmentList::default());
        protocol.components.insert(
            tag.into(),
            ComponentData {
                css: "p{color:red}".into(),
                ..Default::default()
            },
        );
    }
    protocol.populate_style_closures(&["entry"]);
    assert_eq!(
        protocol.style_closure("entry").expect("closure"),
        ["x-first", "x-second", "x-after"]
    );
}
