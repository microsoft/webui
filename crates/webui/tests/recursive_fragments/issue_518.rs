// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use serde_json::json;
use webui::Protocol;
use webui_protocol::web_ui_fragment::Fragment;

use super::{build_app, render, webui_options};

#[test]
fn recursive_tree_renders_once_with_webui_and_missing_leaf_children() {
    let tree = concat!(
        r#"<fragment name="tree-items"><ul><for each="{{child in items}}">"#,
        r#"<li><span>{{child.name}}</span><if condition="{{child.children.length}}">"#,
        r#"<render fragment="tree-items" scope="{{child.children}}" as="items"></render>"#,
        r#"</if></li></for></ul></fragment>"#,
        r#"<render fragment="tree-items" scope="{{items}}" as="items"></render>"#,
    );
    let result = build_app(
        &[
            (
                "index.html",
                "<html><head></head><body><recursive-tree></recursive-tree></body></html>",
            ),
            ("recursive-tree.html", tree),
            ("recursive-tree.ts", "export {};"),
        ],
        webui_options(),
    );
    let targets: Vec<_> = result
        .protocol
        .fragments
        .values()
        .flat_map(|record| &record.fragments)
        .filter_map(|fragment| match fragment.fragment.as_ref() {
            Some(Fragment::Render(call)) => Some(call.fragment_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(targets.len(), 2);
    assert_eq!(targets[0], targets[1]);

    let protocol = Protocol::from_protobuf(&result.protocol_bytes)
        .unwrap_or_else(|error| panic!("recursive tree decode: {error}"));
    let state = json!({
        "items": [
            {"name": "Colors", "children": [
                {"name": "Ali"}, {"name": "Alice"}, {"name": "Bob"}
            ]},
            {"name": "Name", "children": []},
            {"name": "Hobbies", "children": [
                {"name": "Sports", "children": [
                    {"name": "Futbol"}, {"name": "Cricket"}
                ]}
            ]}
        ]
    });
    let html = render(&protocol, &state, true, "/");
    let (body, _) = html
        .split_once(r#"<script type="application/json" id="webui-data""#)
        .unwrap_or_else(|| panic!("WebUI bootstrap is missing"));
    assert_eq!(body.matches("</ul>").count(), 4);
    assert_eq!(body.matches("</li>").count(), 9);
    for name in [
        ">Colors<",
        ">Ali<",
        ">Alice<",
        ">Bob<",
        ">Name<",
        ">Hobbies<",
        ">Sports<",
        ">Futbol<",
        ">Cricket<",
    ] {
        assert_eq!(body.matches(name).count(), 1, "{name}");
    }
    assert!(!body.contains("<fragment"));
    assert!(!body.contains("<render"));
}
