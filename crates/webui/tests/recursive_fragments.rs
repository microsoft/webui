// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Named-fragment coverage through the public build, wire, and server APIs.

use std::fs;
use std::sync::Arc;

use serde_json::{json, Value};
use webui::{
    build, BoundaryMode, BuildOptions, BuildResult, CssStrategy, DomStrategy, Plugin, Protocol,
    RenderOptions, SessionOptions, StreamingSession, WebUIHandler, WebUIProtocol,
};
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_protocol::web_ui_fragment::Fragment;

#[path = "recursive_fragments/issue_518.rs"]
mod issue_518;

const TREE: &str = concat!(
    r#"<fragment name="heading"><h2>{{title}}</h2></fragment>"#,
    r#"<fragment name="tree-items"><ul><for each="child in items">"#,
    r#"<li><span>{{child.name}}</span><if condition="child.children.length">"#,
    r#"<render fragment="tree-items" scope="{{child.children}}" as="items"></render>"#,
    r#"</if></li></for></ul></fragment>"#,
    r#"<render fragment="heading"></render>"#,
    r#"<render fragment="tree-items" scope="{{items}}" as="items"></render>"#,
);

webui_handler::define_string_response_writer!(CaptureWriter, html);

fn build_app(files: &[(&str, &str)], options: BuildOptions) -> BuildResult {
    try_build_app(files, options).unwrap_or_else(|error| panic!("fragment fixture build: {error}"))
}

fn try_build_app(
    files: &[(&str, &str)],
    mut options: BuildOptions,
) -> Result<BuildResult, webui::WebUIError> {
    let directory = tempfile::Builder::new()
        .prefix(".fragment-integration-")
        .tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .unwrap_or_else(|error| panic!("fixture directory: {error}"));
    for (name, source) in files {
        fs::write(directory.path().join(name), source)
            .unwrap_or_else(|error| panic!("fixture {name}: {error}"));
    }
    options.app_dir = directory.path().to_path_buf();
    build(options)
}

fn webui_options() -> BuildOptions {
    BuildOptions {
        plugin: Some(Plugin::WebUI),
        css: CssStrategy::Style,
        ..BuildOptions::default()
    }
}

fn state() -> Value {
    json!({
        "title": "Tree",
        "items": [
            {"name": "Oak", "children": [
                {"name": "Leaf & bud", "children": []}
            ]},
            {"name": "Pine", "children": []}
        ]
    })
}

fn render(protocol: &Protocol, state: &Value, plugin: bool, path: &str) -> String {
    let handler = if plugin {
        WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()))
    } else {
        WebUIHandler::new()
    };
    let mut writer = CaptureWriter::with_capacity(4096);
    handler
        .render(
            protocol,
            state,
            &RenderOptions::new("index.html", path),
            &mut writer,
        )
        .unwrap_or_else(|error| panic!("fragment render: {error}"));
    writer.html
}

#[test]
fn recursive_entry_roundtrips_and_renders_without_a_plugin() {
    let result = build_app(&[("index.html", TREE)], BuildOptions::default());
    let decoded = WebUIProtocol::from_protobuf(&result.protocol_bytes)
        .unwrap_or_else(|error| panic!("decode: {error}"));
    assert_eq!(decoded, result.protocol);
    assert!(decoded.fragments.values().any(|record| record
        .fragments
        .iter()
        .any(|fragment| matches!(fragment.fragment, Some(Fragment::Render(_))))));
    let protocol = Protocol::from_protobuf(&result.protocol_bytes)
        .unwrap_or_else(|error| panic!("runtime decode: {error}"));
    assert_eq!(
        render(&protocol, &state(), false, "/"),
        "<h2>Tree</h2><ul><li><span>Oak</span><ul><li><span>Leaf &amp; bud</span></li></ul></li><li><span>Pine</span></li></ul>"
    );
}

#[test]
fn fast_builds_reject_named_fragments_instead_of_emitting_inert_directives() {
    for plugin in [Plugin::FastV2, Plugin::FastV3] {
        let result = try_build_app(
            &[("index.html", TREE)],
            BuildOptions {
                plugin: Some(plugin),
                ..BuildOptions::default()
            },
        );
        let error = result
            .err()
            .unwrap_or_else(|| panic!("FAST must reject fragment syntax"));
        let message = error.chain_message();
        assert!(
            message.contains("unsupported-fragment-directive"),
            "{plugin:?}: {message}"
        );
    }
}

#[test]
fn call_frames_isolate_caller_loops_and_aliases_but_share_owner_state() {
    let source = concat!(
        r#"<fragment name="owner"><p>{{row.name}}/{{title}}</p></fragment>"#,
        r#"<fragment name="scoped"><b>{{row.name}}</b><render fragment="owner"></render></fragment>"#,
        r#"<for each="row in items"><render fragment="scoped" scope="{{row}}" as="row"></render></for>"#,
    );
    let result = build_app(&[("index.html", source)], BuildOptions::default());
    let protocol = Protocol::from_protobuf(&result.protocol_bytes)
        .unwrap_or_else(|error| panic!("decode: {error}"));
    let html = render(
        &protocol,
        &json!({
            "title": "Owner",
            "row": {"name": "root"},
            "items": [{"name": "first"}, {"name": "second"}]
        }),
        false,
        "/",
    );
    assert_eq!(
        html,
        "<b>first</b><p>root/Owner</p><b>second</b><p>root/Owner</p>"
    );
}

#[test]
fn synthetic_length_descendants_are_missing_render_inputs() {
    let source = concat!(
        r#"<fragment name="value"><p>{{value}}</p></fragment>"#,
        r#"<render fragment="value" scope="{{source.length.more}}" as="value"></render>"#,
    );
    let result = build_app(&[("index.html", source)], BuildOptions::default());
    let protocol = Protocol::from_protobuf(&result.protocol_bytes)
        .unwrap_or_else(|error| panic!("decode: {error}"));
    for source in [json!("é😀"), json!([1, 2])] {
        let mut writer = CaptureWriter::with_capacity(64);
        let error = WebUIHandler::new().render(
            &protocol,
            &json!({"source": source}),
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        );
        assert!(
            error.is_err(),
            "a synthetic scalar length has no descendants"
        );
    }
}

#[test]
fn named_fragment_string_length_inputs_preserve_utf8_byte_counts() {
    let source = concat!(
        r#"<fragment name="value"><p>{{value}}</p></fragment>"#,
        r#"<render fragment="value" scope="{{source.length}}" as="value"></render>"#,
    );
    let result = build_app(&[("index.html", source)], BuildOptions::default());
    let protocol = Protocol::from_protobuf(&result.protocol_bytes)
        .unwrap_or_else(|error| panic!("decode: {error}"));
    for (source, expected) in [
        ("é", "<p>2</p>"),
        ("😀", "<p>4</p>"),
        ("é😀", "<p>6</p>"),
        ("e\u{0301}", "<p>3</p>"),
    ] {
        assert_eq!(
            render(&protocol, &json!({"source": source}), false, "/"),
            expected
        );
    }
}

#[test]
fn ordinary_ssr_allows_256_calls_independent_of_structural_frames() {
    let source = concat!(
        r#"<fragment name="walk"><i>{{node.name}}</i><if condition="node.next">"#,
        r#"<render fragment="walk" scope="{{node.next}}" as="node"></render>"#,
        r#"</if></fragment><render fragment="walk" scope="{{tree}}" as="node"></render>"#,
    );
    let result = build_app(&[("index.html", source)], BuildOptions::default());
    let protocol = Protocol::from_protobuf(&result.protocol_bytes)
        .unwrap_or_else(|error| panic!("decode: {error}"));
    let mut node = json!({"name": "leaf", "next": null});
    for _ in 1..256 {
        node = json!({"name": "branch", "next": node});
    }
    let state = json!({"tree": node});
    assert_eq!(
        render(&protocol, &state, false, "/").matches("<i>").count(),
        256
    );
}

#[test]
fn component_graph_metadata_and_resources_survive_the_wire() {
    let result = build_app(
        &[
            (
                "index.html",
                "<html><head></head><body><tree-view></tree-view></body></html>",
            ),
            ("tree-view.html", TREE),
            ("tree-view.ts", "export {};"),
            ("tree-view.css", "ul { color: green; }"),
        ],
        webui_options(),
    );
    let protocol = Protocol::from_protobuf(&result.protocol_bytes)
        .unwrap_or_else(|error| panic!("decode: {error}"));
    let component = &result.protocol.components["tree-view"];
    let metadata: Value = serde_json::from_str(&component.template_json)
        .unwrap_or_else(|error| panic!("metadata: {error}"));
    assert_eq!(metadata["u"].as_array().map(Vec::len), Some(2));
    assert!(metadata["b"]
        .as_array()
        .is_some_and(|blocks| !blocks.is_empty()));
    let html = render(&protocol, &state(), true, "/");
    assert!(html.contains("<!--wf-->"));
    assert_eq!(
        html.matches("<!--wf-->").count(),
        html.matches("<!--/wf-->").count()
    );
    assert_eq!(html.matches(">Oak<").count(), 1);
    assert_eq!(html.matches(">Leaf &amp; bud<").count(), 1);
    assert!(!html.contains("<fragment"));
    assert!(!html.contains("<render"));
    assert!(!html.contains("\"fragmentInputs\""));
    assert!(!html.contains("\"fragmentSources\""));
    assert!(!html.contains("\"fragmentSourceRefs\""));
    assert!(!html.contains("<!--wf:"));
    let templates = protocol
        .render_component_templates(&["tree-view"], "")
        .unwrap_or_else(|error| panic!("component templates: {error}"));
    assert!(templates["templates"]["tree-view"]["b"].is_array());
    let inventory = templates["inventory"].as_str().unwrap_or_default();
    let known = protocol
        .render_component_templates(&["tree-view"], inventory)
        .unwrap_or_else(|error| panic!("known component templates: {error}"));
    assert_eq!(
        known["templates"].as_object().map(|value| value.len()),
        Some(0)
    );
}

#[test]
fn render_markers_share_the_compiler_implied_table_container() {
    let component = concat!(
        r#"<fragment name="row"><tr><td>{{title}}</td></tr></fragment>"#,
        r#"<fragment name="column"><col></fragment>"#,
        r#"<table><render fragment="column"></render><render fragment="row"></render></table>"#,
    );
    let result = build_app(
        &[
            (
                "index.html",
                "<html><head></head><body><table-view></table-view></body></html>",
            ),
            ("table-view.html", component),
            ("table-view.ts", "export {};"),
        ],
        webui_options(),
    );
    let protocol = Protocol::from_protobuf(&result.protocol_bytes)
        .unwrap_or_else(|error| panic!("decode: {error}"));
    let html = render(&protocol, &json!({"title": "Cell"}), true, "/");
    assert!(html.contains("<colgroup><!--wf--><col><!--/wf--></colgroup>"));
    assert!(html.contains("<tbody><!--wf--><tr>"));
    assert!(html.contains("</tr><!--/wf--></tbody>"));
}

#[test]
fn recursive_route_partial_includes_nested_components_once() {
    let tree = concat!(
        r#"<fragment name="tree-items"><for each="child in items">"#,
        r#"<tree-label label="{{child.name}}"></tree-label>"#,
        r#"<if condition="child.children.length">"#,
        r#"<render fragment="tree-items" scope="{{child.children}}" as="items"></render>"#,
        r#"</if></for></fragment>"#,
        r#"<render fragment="tree-items" scope="{{items}}" as="items"></render>"#,
    );
    let result = build_app(
        &[
            (
                "index.html",
                r#"<html><head></head><body><route path="/tree" component="tree-view" exact></route></body></html>"#,
            ),
            ("tree-view.html", tree),
            ("tree-view.ts", "export {};"),
            ("tree-label.html", "<span>{{label}}</span>"),
            ("tree-label.css", "span { color: green; }"),
        ],
        webui_options(),
    );
    let protocol = Protocol::from_protobuf(&result.protocol_bytes)
        .unwrap_or_else(|error| panic!("decode: {error}"));
    let partial: Value = serde_json::from_str(
        &protocol
            .render_partial(state(), "index.html", "/tree", "")
            .unwrap_or_else(|error| panic!("partial: {error}")),
    )
    .unwrap_or_else(|error| panic!("partial JSON: {error}"));
    assert_eq!(
        partial["templates"].as_object().map(|value| value.len()),
        Some(2)
    );
    assert!(partial["templates"]["tree-label"].is_object());
    assert_eq!(partial["state"]["items"], state()["items"]);
    let html = render(&protocol, &state(), true, "/tree");
    let rendered_html = html
        .split_once(r#"<script type="application/json" id="webui-data""#)
        .map_or(html.as_str(), |(rendered, _)| rendered);
    assert_eq!(rendered_html.matches("<tree-label").count(), 3);
    assert_eq!(html.matches(">Leaf &amp; bud<").count(), 1);
}

#[test]
fn recursive_component_assets_are_deterministic_and_keep_entry_calls() {
    let entry = concat!(
        "<html><head></head><body>",
        r#"<fragment name="cards"><entry-card></entry-card></fragment>"#,
        r#"<render fragment="cards"></render><render fragment="cards"></render>"#,
        "</body></html>",
    );
    let tree = concat!(
        r#"<fragment name="walk"><tree-label></tree-label><if condition="next">"#,
        r#"<render fragment="walk"></render></if></fragment>"#,
        r#"<fragment name="unused"><unused-card></unused-card></fragment>"#,
        r#"<render fragment="walk"></render>"#,
    );
    let files = [
        ("index.html", entry),
        ("entry-card.html", "<p>entry</p>"),
        ("entry-card.css", "p { color: navy; }"),
        ("tree-view.html", tree),
        ("tree-view.css", "tree-view { display: block; }"),
        ("tree-label.html", "<p>label</p>"),
        ("tree-label.css", "p { color: green; }"),
        ("unused-card.html", "<aside>unused</aside>"),
        ("unused-card.css", "aside { color: red; }"),
    ];
    let options = || BuildOptions {
        plugin: Some(Plugin::WebUI),
        dom: DomStrategy::Light,
        component_asset_roots: vec!["tree-view".to_string()],
        metafile: true,
        ..BuildOptions::default()
    };
    let first = build_app(&files, options());
    let second = build_app(&files, options());
    assert_eq!(first.component_asset_files, second.component_asset_files);
    assert_eq!(first.metafile, second.metafile);
    assert_eq!(first.protocol.component_asset_style_preloads.len(), 1);
    assert_eq!(
        first.protocol.component_asset_style_preloads[0].style_hrefs,
        ["tree-view.css", "tree-label.css"]
    );
    assert!(!first
        .css_files
        .iter()
        .any(|(name, _)| name.contains("unused")));
    let asset = first
        .component_asset_files
        .iter()
        .map(|file| file.content.as_str())
        .collect::<String>();
    assert!(asset.contains("tree-label"));
    assert!(!asset.contains("unused-card"));
    let protocol = Protocol::from_protobuf(&first.protocol_bytes)
        .unwrap_or_else(|error| panic!("decode: {error}"));
    assert_eq!(
        render(&protocol, &json!({}), true, "/")
            .matches("<entry-card")
            .count(),
        2
    );
}

#[test]
fn streaming_retains_call_alias_while_resume_replaces_owning_state() {
    let entry = concat!(
        "<html><head></head><body>",
        r#"<fragment name="card"><boundary name="ready" key="{{row.id}}">"#,
        r#"<p>{{row.name}}/{{title}}</p></boundary><i>{{row.name}}</i></fragment>"#,
        r#"<render fragment="card" scope="{{payload}}" as="row"></render>"#,
        r#"<render fragment="card" scope="{{payload}}" as="row"></render>"#,
        "<footer>{{title}}</footer></body></html>",
    );
    let result = build_app(&[("index.html", entry)], BuildOptions::default());
    let protocol = Protocol::from_protobuf(&result.protocol_bytes)
        .unwrap_or_else(|error| panic!("decode: {error}"));
    let mut session = StreamingSession::new(
        Arc::new(WebUIHandler::new()),
        Arc::new(protocol),
        SessionOptions::new("index.html", "/"),
    )
    .unwrap_or_else(|error| panic!("session: {error}"));
    let first = session
        .start(json!({"payload": {"id": "first", "name": "captured"}, "title": "before"}))
        .unwrap_or_else(|error| panic!("start: {error}"));
    let first = first
        .boundary
        .unwrap_or_else(|| panic!("first boundary missing"));
    let committed = session
        .resume(
            first.instance_id,
            json!({"payload": {"id": "second", "name": "replacement"}, "title": "resumed"}),
            BoundaryMode::Final,
        )
        .unwrap_or_else(|error| panic!("first resume: {error}"));
    let committed =
        String::from_utf8(committed.bytes).unwrap_or_else(|error| panic!("commit UTF-8: {error}"));
    assert!(committed.contains("<p>captured/resumed</p>"));
    assert!(!committed.contains("<i>"));
    let second = session
        .advance()
        .unwrap_or_else(|error| panic!("advance: {error}"));
    assert!(String::from_utf8_lossy(&second.bytes).contains("<i>captured</i>"));
    let second = second
        .boundary
        .unwrap_or_else(|| panic!("second boundary missing"));
    assert_eq!(first.declaration_id, second.declaration_id);
    assert_ne!(first.key, second.key);
    let committed = session
        .resume(
            second.instance_id,
            json!({"payload": {"id": "third", "name": "newest"}, "title": "last"}),
            BoundaryMode::Final,
        )
        .unwrap_or_else(|error| panic!("second resume: {error}"));
    assert!(String::from_utf8_lossy(&committed.bytes).contains("<p>replacement/last</p>"));
    let done = session
        .advance()
        .unwrap_or_else(|error| panic!("finish: {error}"));
    assert!(done.done);
    let tail = String::from_utf8_lossy(&done.bytes);
    assert!(tail.contains("<i>replacement</i>"));
    assert!(tail.contains("<footer>last</footer>"));
}
