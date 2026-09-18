// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::super::parsed_protocol_data;
use super::{
    assert_content_before_host, document, handler, outlet_protocol_with_content, routes,
    FailingSelectedWriter, CHILDREN,
};
use std::sync::Arc;
use webui_handler::{
    BoundaryMode, HandlerError, Protocol, RenderOptions, SessionOptions, StreamingSession,
};
use webui_protocol::web_ui_fragment::Fragment;
use webui_test_utils::test_json;

fn compiled(entry: &str, components: &[(&str, &str)]) -> Arc<Protocol> {
    let data = parsed_protocol_data(&document(entry), components);
    if let Some(carrier) = data.fragments.get("outlet-carrier") {
        assert!(!carrier.contains_boundary);
    }
    Arc::new(Protocol::new(data))
}

fn streamed(protocol: Arc<Protocol>, path: &str) -> String {
    let mut session = StreamingSession::new(
        Arc::new(handler()),
        protocol,
        SessionOptions::new("index.html", path),
    )
    .unwrap();
    let mut chunk = session
        .start(test_json!({"show": true, "rows": [1, 2]}))
        .unwrap();
    let mut output = String::new();
    loop {
        output.push_str(std::str::from_utf8(&chunk.bytes).unwrap());
        if chunk.done {
            return output;
        }
        chunk = match chunk.boundary.as_ref() {
            Some(boundary) => session
                .resume_current(boundary.instance_id, BoundaryMode::Final)
                .unwrap(),
            None => session.advance().unwrap(),
        };
    }
}

#[test]
fn only_boundary_free_component_if_and_for_edges_promote_outlet_order() {
    for (shell, expected) in [
        ("<outlet />", ["a", "b", "c"]),
        ("<outlet-carrier></outlet-carrier>", ["b", "a", "c"]),
        (r#"<if condition="show"><outlet /></if>"#, ["b", "a", "c"]),
        (
            r#"<if condition="!show"><outlet /></if><outlet />"#,
            ["a", "b", "c"],
        ),
        (
            r#"<for each="item in rows"><outlet /></for>"#,
            ["b", "a", "c"],
        ),
        (
            concat!(
                r#"<fragment name="call"><outlet /></fragment>"#,
                r#"<render fragment="call" />"#,
            ),
            ["a", "b", "c"],
        ),
    ] {
        let entry = document(&format!(
            r#"<boundary name="outer"><route path="/" component="outlet-shell">{CHILDREN}</route></boundary>"#
        ));
        let data = parsed_protocol_data(
            &entry,
            &[
                ("outlet-shell", shell),
                ("outlet-carrier", "<outlet />"),
                ("outlet-a", "<p>A</p>"),
                ("outlet-b", "<p>B</p>"),
                ("outlet-c", "<p>C</p>"),
            ],
        );
        assert!(data.fragments["index.html"].contains_boundary);
        assert!(!data.fragments["outlet-shell"].contains_boundary);
        for list in data.fragments.values() {
            for fragment in &list.fragments {
                let target = match fragment.fragment.as_ref() {
                    Some(Fragment::Component(component)) => Some(&component.fragment_id),
                    Some(Fragment::IfCond(condition)) => Some(&condition.fragment_id),
                    Some(Fragment::ForLoop(repeat)) => Some(&repeat.fragment_id),
                    Some(Fragment::Render(render)) => Some(&render.fragment_id),
                    _ => None,
                };
                if let Some(target) = target {
                    assert!(!data.fragments[target].contains_boundary, "{target}");
                }
            }
        }
        let output = streamed(Arc::new(Protocol::new(data)), "/b");
        let found = routes(&output);
        assert_eq!(
            found[1..].iter().map(|route| route.0).collect::<Vec<_>>(),
            expected,
            "{shell}: {output}"
        );
        assert_eq!(found[1..].iter().filter(|route| route.2).count(), 1);
    }
}

#[test]
fn route_content_and_deferred_generated_hosts_do_not_promote_or_leak_policy() {
    for (content, shell, rendered_content) in [
        ("<div><outlet /></div>", "<p>shell</p>", "</div>"),
        (
            r#"<if condition="show"><span>content</span></if>"#,
            "<outlet />",
            "<span>content</span>",
        ),
    ] {
        let protocol = outlet_protocol_with_content(
            &format!(
                r#"<boundary name="outer"><route path="/" component="outlet-shell">{CHILDREN}</route></boundary>"#
            ),
            &[
                ("outlet-shell", shell),
                ("outlet-a", "<p>A</p>"),
                ("outlet-b", "<p>B</p>"),
                ("outlet-c", "<p>C</p>"),
            ],
            "outlet-shell",
            content,
        );
        let output = streamed(protocol, "/b");
        assert_content_before_host(&output, rendered_content, "outlet-shell");
        assert_eq!(
            routes(&output)[1..]
                .iter()
                .map(|route| route.0)
                .collect::<Vec<_>>(),
            ["a", "b", "c"],
            "{output}"
        );
    }
}

#[test]
fn boundary_containing_component_and_if_edges_retain_declaration_order() {
    for shell in [
        "<outlet-live></outlet-live>",
        r#"<if condition="show"><boundary name="inside"><outlet /></boundary></if>"#,
    ] {
        let data = parsed_protocol_data(
            &document(&format!(
                r#"<route path="/" component="outlet-shell">{CHILDREN}</route>"#
            )),
            &[
                ("outlet-shell", shell),
                (
                    "outlet-live",
                    r#"<boundary name="inside"><outlet /></boundary>"#,
                ),
                ("outlet-a", "<p>A</p>"),
                ("outlet-b", "<p>B</p>"),
                ("outlet-c", "<p>C</p>"),
            ],
        );
        assert!(data.fragments["outlet-shell"].contains_boundary);
        let output = streamed(Arc::new(Protocol::new(data)), "/b");
        assert_eq!(
            routes(&output)[1..]
                .iter()
                .map(|route| route.0)
                .collect::<Vec<_>>(),
            ["a", "b", "c"],
            "{output}"
        );
    }
}

#[test]
fn inherited_order_crosses_route_content_generated_hosts_and_render_then_restores() {
    for (content, nested_shell, rendered_content) in [
        ("<div><outlet /></div>", "<p>nested</p>", "</div>"),
        (
            "<span>content</span>",
            concat!(
                r#"<fragment name="call"><outlet /></fragment>"#,
                r#"<render fragment="call" />"#,
            ),
            "<span>content</span>",
        ),
    ] {
        let carrier = format!(r#"<route path="/" component="nested-shell">{CHILDREN}</route>"#);
        let protocol = outlet_protocol_with_content(
            &format!(
                r#"<boundary name="outer"><route path="/" component="outlet-shell">{CHILDREN}</route></boundary>"#
            ),
            &[
                (
                    "outlet-shell",
                    "<outlet-carrier></outlet-carrier><outlet />",
                ),
                ("outlet-carrier", &carrier),
                ("nested-shell", nested_shell),
                ("outlet-a", "<p>A</p>"),
                ("outlet-b", "<p>B</p>"),
                ("outlet-c", "<p>C</p>"),
            ],
            "nested-shell",
            content,
        );
        let output = streamed(protocol, "/b");
        assert_content_before_host(&output, rendered_content, "nested-shell");
        assert_eq!(
            routes(&output),
            [
                ("/", "outlet-shell", true),
                ("/", "nested-shell", true),
                ("b", "outlet-b", true),
                ("a", "outlet-a", false),
                ("c", "outlet-c", false),
                ("a", "outlet-a", false),
                ("b", "outlet-b", true),
                ("c", "outlet-c", false),
            ],
            "{output}"
        );
    }
}

#[test]
fn reused_repeat_records_and_shared_render_targets_keep_entry_specific_policy() {
    let shell = format!(
        concat!(
            r#"<fragment name="group"><route path="/" component="nested-shell">"#,
            "{}",
            r#"</route></fragment><for each="item in rows"><render fragment="group" /></for>"#,
            r#"<render fragment="group" />"#,
        ),
        CHILDREN,
    );
    let protocol = compiled(
        &format!(
            r#"<boundary name="outer"><route path="/" component="outlet-shell">{CHILDREN}</route></boundary>"#
        ),
        &[
            ("outlet-shell", &shell),
            ("nested-shell", "<outlet />"),
            ("outlet-a", "<p>A</p>"),
            ("outlet-b", "<p>B</p>"),
            ("outlet-c", "<p>C</p>"),
        ],
    );
    let output = streamed(protocol, "/b");
    let children = routes(&output)
        .into_iter()
        .filter(|route| route.0 != "/")
        .map(|route| route.0)
        .collect::<Vec<_>>();
    assert_eq!(
        children,
        ["b", "a", "c", "b", "a", "c", "a", "b", "c"],
        "{output}"
    );
}

#[test]
fn inherited_outlet_order_survives_a_selected_route_boundary() {
    let protocol = compiled(
        concat!(
            r#"<route path="/" component="outlet-shell">"#,
            r#"<route path="a" component="outlet-a" exact />"#,
            r#"<route path="b" component="outlet-b">"#,
            r#"<route path="x" component="outlet-a" exact />"#,
            r#"<route path="y" component="outlet-c" exact />"#,
            "</route>",
            r#"<route path="c" component="outlet-c" exact />"#,
            "</route>",
        ),
        &[
            (
                "outlet-shell",
                concat!(
                    "<outlet-carrier></outlet-carrier>",
                    r#"<boundary name="shell-end"><p>shell</p></boundary>"#,
                ),
            ),
            ("outlet-carrier", "<outlet />"),
            ("outlet-a", "<p>A</p>"),
            (
                "outlet-b",
                concat!(
                    r#"<boundary name="leaf"><outlet /></boundary>"#,
                    "<footer>tail</footer>",
                ),
            ),
            ("outlet-c", "<p>C</p>"),
        ],
    );
    let mut session = StreamingSession::new(
        Arc::new(handler()),
        protocol,
        SessionOptions::new("index.html", "/b/y"),
    )
    .unwrap();
    let start = session.start(test_json!({})).unwrap();
    let leaf = start.boundary.unwrap();
    assert_eq!(leaf.name.as_ref(), "leaf");
    let mut output = String::from_utf8(start.bytes).unwrap();
    assert_eq!(
        routes(&output),
        [("/", "outlet-shell", true), ("b", "outlet-b", true)],
        "{output}"
    );
    let commit = session
        .resume_current(leaf.instance_id, BoundaryMode::Final)
        .unwrap();
    output.push_str(&String::from_utf8(commit.bytes).unwrap());
    let next = session.advance().unwrap();
    let shell = next.boundary.unwrap();
    assert_eq!(shell.name.as_ref(), "shell-end");
    output.push_str(&String::from_utf8(next.bytes).unwrap());
    assert_eq!(
        routes(&output),
        [
            ("/", "outlet-shell", true),
            ("b", "outlet-b", true),
            ("y", "outlet-c", true),
            ("x", "outlet-a", false),
            ("a", "outlet-a", false),
            ("c", "outlet-c", false),
        ],
        "{output}"
    );
    let commit = session
        .resume_current(shell.instance_id, BoundaryMode::Final)
        .unwrap();
    output.push_str(&String::from_utf8(commit.bytes).unwrap());
    let tail = session.advance().unwrap();
    assert!(tail.done);
    output.push_str(&String::from_utf8(tail.bytes).unwrap());
    assert_eq!(routes(&output).len(), 6, "{output}");
    assert_eq!(
        output.matches("<footer>tail</footer>").count(),
        1,
        "{output}"
    );
}

#[test]
fn single_call_inherited_outlet_failure_stops_before_hidden_siblings() {
    let protocol = compiled(
        &format!(
            r#"<boundary name="outer"><route path="/" component="outlet-shell">{CHILDREN}</route></boundary>"#
        ),
        &[
            ("outlet-shell", "<outlet-carrier></outlet-carrier>"),
            ("outlet-carrier", "<outlet />"),
            ("outlet-a", "<p>A</p>"),
            ("outlet-b", "<p>SELECTED</p>"),
            ("outlet-c", "<p>C</p>"),
        ],
    );
    let mut writer = FailingSelectedWriter::default();
    assert!(matches!(
        handler().render_streaming(
            &protocol,
            &test_json!({}),
            &RenderOptions::new("index.html", "/b"),
            &mut writer,
        ),
        Err(HandlerError::ClientDisconnected)
    ));
    assert!(writer.failed);
    assert_eq!(writer.writes_after_failure, 0);
    assert_eq!(
        routes(&writer.output),
        [("/", "outlet-shell", true), ("b", "outlet-b", true)],
        "{}",
        writer.output
    );
}
