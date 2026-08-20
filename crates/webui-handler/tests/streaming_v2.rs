// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use std::sync::Arc;

use webui_handler::{
    BoundaryInstanceId, BoundaryKey, BoundaryMode, FlushWriter, HandlerError, Protocol,
    RenderOptions, ResponseWriter, SessionOptions, StreamingSession, WebUIHandler,
};
use webui_parser::{ComponentRegistration, HtmlParser};
use webui_protocol::{ComponentData, InitialStateStrategy, StateProjectionMode, WebUIProtocol};
use webui_test_utils::test_json;

fn parsed_protocol(entry: &str, components: &[(&str, &str)]) -> Arc<Protocol> {
    Arc::new(Protocol::new(parsed_protocol_data(entry, components)))
}

fn parsed_protocol_data(entry: &str, components: &[(&str, &str)]) -> WebUIProtocol {
    let mut parser = HtmlParser::new();
    for (tag, template) in components {
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(tag, template, None, true))
            .unwrap();
    }
    parser.parse("index.html", entry).unwrap();
    WebUIProtocol::new(parser.into_fragment_records())
}

fn new_session(protocol: Arc<Protocol>, path: &str) -> StreamingSession {
    StreamingSession::new(
        Arc::new(WebUIHandler::new()),
        protocol,
        SessionOptions::new("index.html", path),
    )
    .unwrap()
}

fn document(body: &str) -> String {
    format!("<html><head></head><body>{body}</body></html>")
}

#[test]
fn component_local_boundary_suspends_and_emits_v2_span_contract() {
    let protocol = parsed_protocol(
        &document(r#"<shell-card title="frozen"></shell-card>"#),
        &[
            (
                "shell-card",
                concat!(
                    "<section>",
                    r#"<boundary name="inside"><child-box label="{{title}}"></child-box></boundary>"#,
                    "<footer>tail</footer></section>",
                ),
            ),
            ("child-box", "<span>{{label}}</span>"),
        ],
    );
    let mut session = new_session(protocol, "/");

    let start = session.start(&test_json!({})).unwrap();
    let boundary = start.boundary.unwrap();
    assert_eq!(boundary.name.as_ref(), "inside");
    assert_eq!(boundary.owner.as_ref(), "shell-card");
    let shell = String::from_utf8(start.bytes).unwrap();
    assert!(shell.contains(r#"<!--ws:0--><shell-card title="frozen" data-ws data-ws-span="0">"#));
    assert!(!shell.contains("<!--wb:0-->"));

    let committed = session
        .resume(boundary.instance_id, &test_json!({}), BoundaryMode::Final)
        .unwrap();
    assert!(!committed.done && committed.boundary.is_none());
    let html = String::from_utf8(committed.bytes).unwrap();
    assert!(html.contains(r#"<!--wb:0--><child-box label="frozen" data-ws data-ws-enclosing="0">"#));
    assert!(html.contains(r#"<!--/wb:0--><script type="application/json" data-webui-boundary>[2,0,0,0,{"declarationId":0,"enclosingSpanInstanceId":0"#));
    // The committed step stops at its checkpoint: the component tail and the
    // span completion that follow belong to the next step.
    assert!(!html.contains("<footer>tail</footer>"));
    assert!(!html.contains("<!--/ws:0-->"));

    let end = session.advance().unwrap();
    assert!(end.done);
    let tail = String::from_utf8(end.bytes).unwrap();
    assert!(tail.contains("<footer>tail</footer>"));
    assert!(tail.contains(
        r#"</shell-card><!--/ws:0--><script type="application/json" data-webui-boundary>[2,1,3,0,"#
    ));
    assert!(tail.contains("[2,2,4,0,{}]"));
    assert!(tail.contains("</script><webui-hydrate></webui-hydrate>"));
}

#[test]
fn resume_writes_only_the_committed_boundary_and_advance_writes_the_tail() {
    let protocol = parsed_protocol(
        &document(concat!(
            r#"<boundary name="search"><p>results</p></boundary>"#,
            "<footer>tail</footer>",
        )),
        &[],
    );
    let mut session = new_session(protocol, "/");

    let start = session.start(&test_json!({})).unwrap();
    let boundary = start.boundary.unwrap();
    assert_eq!(boundary.name.as_ref(), "search");

    let committed = session
        .resume(boundary.instance_id, &test_json!({}), BoundaryMode::Final)
        .unwrap();
    assert!(!committed.done && committed.boundary.is_none());
    let bytes = String::from_utf8(committed.bytes).unwrap();
    // Exact segment: the occurrence markers, its body, its record, and nothing
    // past the record's hydration sentinel.
    assert!(bytes.starts_with("<!--wb:0--><p>results</p><!--/wb:0--><script"));
    assert!(bytes.ends_with("<webui-hydrate></webui-hydrate>"));
    assert!(!bytes.contains("<footer>tail</footer>"));
    assert!(!bytes.contains("[2,1,4,0,{}]"));

    let tail = session.advance().unwrap();
    assert!(tail.done);
    let tail = String::from_utf8(tail.bytes).unwrap();
    assert!(tail.starts_with("<footer>tail</footer>"));
    assert!(tail.contains("[2,1,4,0,{}]"));
    assert!(!tail.contains("<!--wb:0-->"));
}

#[test]
fn multiple_boundaries_alternate_descriptor_commit_and_advance() {
    let protocol = parsed_protocol(
        &document(concat!(
            r#"<boundary name="first"><p>1</p></boundary>"#,
            "<hr>",
            r#"<boundary name="second"><p>2</p></boundary>"#,
            "<footer>tail</footer>",
        )),
        &[],
    );
    let mut session = new_session(protocol, "/");

    let start = session.start(&test_json!({})).unwrap();
    let first = start.boundary.clone().unwrap();
    assert_eq!(first.instance_id.raw(), 0);

    let committed = session
        .resume(first.instance_id, &test_json!({}), BoundaryMode::Final)
        .unwrap();
    assert!(committed.boundary.is_none() && !committed.done);

    let between = session.advance().unwrap();
    let second = between.boundary.clone().unwrap();
    assert_eq!(second.instance_id.raw(), 1);
    let between = String::from_utf8(between.bytes).unwrap();
    assert!(between.starts_with("<hr>"), "{between}");
    assert!(!between.contains("<!--wb:1-->"));

    let committed = session
        .resume(second.instance_id, &test_json!({}), BoundaryMode::Final)
        .unwrap();
    assert!(committed.boundary.is_none() && !committed.done);
    assert!(String::from_utf8(committed.bytes)
        .unwrap()
        .contains("<!--wb:1--><p>2</p><!--/wb:1-->"));

    let end = session.advance().unwrap();
    assert!(end.done && end.boundary.is_none());
    assert!(String::from_utf8(end.bytes)
        .unwrap()
        .contains("<footer>tail</footer>"));
    assert!(session.is_done());
}

#[test]
fn out_of_order_steps_are_rejected_without_poisoning() {
    let protocol = parsed_protocol(
        &document(concat!(
            r#"<boundary name="first"><p>1</p></boundary>"#,
            r#"<boundary name="second"><p>2</p></boundary>"#,
        )),
        &[],
    );
    let mut session = new_session(protocol, "/");

    // advance before any commit is an ordering error, and writes nothing.
    assert!(session
        .advance()
        .unwrap_err()
        .to_string()
        .contains("start must be called before this operation"));

    let first = session.start(&test_json!({})).unwrap().boundary.unwrap();
    assert!(session
        .advance()
        .unwrap_err()
        .to_string()
        .contains("no committed boundary to advance past"));

    let committed = session
        .resume(first.instance_id, &test_json!({}), BoundaryMode::Final)
        .unwrap();
    assert!(committed.boundary.is_none());

    // Resuming again before advancing is rejected, and the response stays
    // usable: the very next advance still produces the second descriptor.
    assert!(session
        .resume(first.instance_id, &test_json!({}), BoundaryMode::Final)
        .unwrap_err()
        .to_string()
        .contains("has not been advanced past"));
    let second = session.advance().unwrap().boundary.unwrap();
    assert_eq!(second.instance_id.raw(), 1);
    assert!(session
        .resume(second.instance_id, &test_json!({}), BoundaryMode::Final)
        .unwrap()
        .boundary
        .is_none());
    assert!(session.advance().unwrap().done);
    assert!(session
        .advance()
        .unwrap_err()
        .to_string()
        .contains("already completed"));
}

#[test]
fn boundary_in_repeat_is_rejected_at_build_time() {
    let mut parser = HtmlParser::new();
    let error = parser
        .parse(
            "index.html",
            &document(
                r#"<for each="item in items"><boundary name="row" key="{{item.id}}"><p>{{item.label}}</p></boundary></for>"#,
            ),
        )
        .expect_err("a boundary inside a repeat must be rejected before render");
    assert!(
        error.to_string().contains("<for>"),
        "the diagnostic must name the repeat: {error}"
    );
}

#[test]
fn boundary_free_repeat_renders_atomically_inside_one_boundary() {
    // The whole finite list is one atomic checkpoint: it renders in the commit
    // step and leaves no resumable repeat state behind.
    let protocol = parsed_protocol(
        &document(concat!(
            r#"<boundary name="feed"><for each="item in items"><p>{{item.label}}</p></for></boundary>"#,
            "<footer>tail</footer>",
        )),
        &[],
    );
    let mut session = new_session(protocol, "/");
    let state = test_json!({ "items": [{ "label": "a" }, { "label": "b" }, { "label": "c" }] });

    let boundary = session.start(&state).unwrap().boundary.unwrap();
    let committed = session
        .resume(boundary.instance_id, &state, BoundaryMode::Final)
        .unwrap();
    assert!(committed.boundary.is_none() && !committed.done);
    let html = String::from_utf8(committed.bytes).unwrap();
    assert!(html.contains("<p>a</p><p>b</p><p>c</p>"), "{html}");
    assert!(!html.contains("<footer>tail</footer>"));

    let end = session.advance().unwrap();
    assert!(end.done);
    assert!(String::from_utf8(end.bytes)
        .unwrap()
        .contains("<footer>tail</footer>"));
}

#[test]
fn nested_repeats_and_components_inside_a_boundary_keep_loop_locals_and_capture() {
    // Nested repeats exercise the frame-driven walk (no recursion) and a
    // component inside a repeat body must still land in the enclosing
    // checkpoint's capture, so its template arrives with that record.
    let protocol = parsed_protocol(
        &document(
            r#"<boundary name="grid"><for each="row in rows"><section>{{row.name}}<for each="cell in row.cells"><cell-box label="{{cell}}" owner="{{row.name}}"></cell-box></for></section></for></boundary>"#,
        ),
        &[("cell-box", "<span>{{owner}}:{{label}}</span>")],
    );
    let mut session = new_session(protocol, "/");
    let state = test_json!({
        "rows": [
            { "name": "r1", "cells": ["a", "b"] },
            { "name": "r2", "cells": ["c"] }
        ]
    });

    let boundary = session.start(&state).unwrap().boundary.unwrap();
    let html = String::from_utf8(
        session
            .resume(boundary.instance_id, &state, BoundaryMode::Final)
            .unwrap()
            .bytes,
    )
    .unwrap();
    // The outer loop local stays visible inside the inner loop body, and the
    // inner local does not leak between rows.
    assert!(html.contains("<span>r1:a</span>"), "{html}");
    assert!(html.contains("<span>r1:b</span>"), "{html}");
    assert!(html.contains("<span>r2:c</span>"), "{html}");
    assert_eq!(html.matches("<span>").count(), 3);
    // The repeated component is captured by the enclosing checkpoint, so its
    // inventory bit rides that record.
    assert!(html.contains(r#""inventory":"01""#), "{html}");
    assert!(session.advance().unwrap().done);
}

#[test]
fn keyed_component_boundary_returns_gapless_descriptors_and_rejects_duplicates() {
    // `<for>` can no longer repeat a declaration, but two static callsites of a
    // boundary-bearing component still can, so keys stay live.
    let protocol = parsed_protocol(
        &document(
            r#"<row-item row-id="{{first}}"></row-item><row-item row-id="{{second}}"></row-item>"#,
        ),
        &[(
            "row-item",
            r#"<boundary name="row" key="{{rowId}}"><p>{{rowId}}</p></boundary>"#,
        )],
    );
    let mut session = new_session(Arc::clone(&protocol), "/");
    let state = test_json!({ "first": 10, "second": 20 });

    let first = session.start(&state).unwrap().boundary.unwrap();
    assert_eq!(first.instance_id.raw(), 0);
    assert_eq!(first.key, Some(BoundaryKey::Number(10.into())));
    assert!(session
        .resume(first.instance_id, &state, BoundaryMode::Final)
        .unwrap()
        .boundary
        .is_none());

    let second = session.advance().unwrap().boundary.unwrap();
    assert_eq!(second.instance_id.raw(), 1);
    assert_eq!(second.key, Some(BoundaryKey::Number(20.into())));
    assert!(session
        .resume(second.instance_id, &state, BoundaryMode::Final)
        .unwrap()
        .boundary
        .is_none());
    assert!(session.advance().unwrap().done);

    let mut duplicate = new_session(protocol, "/");
    let duplicate_state = test_json!({ "first": 7, "second": 7 });
    let first = duplicate.start(&duplicate_state).unwrap().boundary.unwrap();
    assert!(duplicate
        .resume(first.instance_id, &duplicate_state, BoundaryMode::Final)
        .unwrap()
        .boundary
        .is_none());
    let error = duplicate.advance().unwrap_err();
    assert!(error.to_string().contains("duplicate key"));
    assert!(duplicate
        .advance()
        .unwrap_err()
        .to_string()
        .contains("poisoned"));
}

#[test]
fn resume_overlays_boundary_state_on_frozen_parent_and_lexical_locals() {
    let protocol = parsed_protocol(
        &document(r#"<row-item label="local"></row-item>"#),
        &[(
            "row-item",
            r#"<boundary name="row"><p>{{label}}/{{global}}</p></boundary>"#,
        )],
    );
    let mut session = new_session(protocol, "/");
    let first = session
        .start(&test_json!({ "global": "frozen" }))
        .unwrap()
        .boundary
        .unwrap();
    let committed = session
        .resume(
            first.instance_id,
            &test_json!({ "global": "boundary" }),
            BoundaryMode::Final,
        )
        .unwrap();
    assert!(String::from_utf8(committed.bytes)
        .unwrap()
        .contains("<p>local/boundary</p>"));
    assert!(session.advance().unwrap().done);
}

#[test]
fn false_if_discovers_no_occurrence_and_boundary_free_start_completes() {
    let protocol = parsed_protocol(
        &document(
            r#"<if condition="show"><boundary name="hidden"><p>no</p></boundary></if><p>done</p>"#,
        ),
        &[],
    );
    let mut session = new_session(protocol, "/");
    let step = session.start(&test_json!({ "show": false })).unwrap();

    assert!(step.done);
    assert!(step.boundary.is_none());
    let html = String::from_utf8(step.bytes).unwrap();
    assert!(!html.contains("<!--wb:"));
    assert!(html.contains("<p>done</p>"));
    assert!(html.contains("[2,0,4,0,{}]"));
}

#[test]
fn component_false_if_emits_generated_span_without_occurrence() {
    let protocol = parsed_protocol(
        &document(r#"<conditional-card show="{{show}}"></conditional-card>"#),
        &[(
            "conditional-card",
            r#"<if condition="show"><boundary name="ready"><p>yes</p></boundary></if>"#,
        )],
    );
    let mut hidden = new_session(Arc::clone(&protocol), "/");
    let hidden = hidden.start(&test_json!({ "show": false })).unwrap();
    assert!(hidden.done);
    let hidden_html = String::from_utf8(hidden.bytes).unwrap();
    assert!(!hidden_html.contains("<!--wb:"));
    assert!(hidden_html.contains("<!--ws:0--><conditional-card"));
    assert!(hidden_html.contains(r#"data-ws data-ws-span="0">"#));
    assert!(hidden_html.contains(
        r#"</conditional-card><!--/ws:0--><script type="application/json" data-webui-boundary>[2,0,3,0,"#
    ));
    assert!(hidden_html.contains("[2,1,4,0,{}]"));

    let mut shown = new_session(protocol, "/");
    let shown = shown.start(&test_json!({ "show": true })).unwrap();
    assert!(shown.boundary.is_some());
    assert!(String::from_utf8(shown.bytes)
        .unwrap()
        .contains("<!--ws:0--><conditional-card"));
}

#[test]
fn route_content_boundary_occurs_only_for_selected_route() {
    let entry = document(concat!(
        r#"<route path="/home" component="home-page"><boundary name="home-ready"><p>home</p></boundary></route>"#,
        r#"<route path="/about" component="about-page"><boundary name="about-ready"><p>about</p></boundary></route>"#,
    ));
    let protocol = parsed_protocol(
        &entry,
        &[
            ("home-page", "<h1>Home</h1>"),
            ("about-page", "<h1>About</h1>"),
        ],
    );

    let home = new_session(Arc::clone(&protocol), "/home")
        .start(&test_json!({}))
        .unwrap()
        .boundary
        .unwrap();
    assert_eq!(home.name.as_ref(), "home-ready");
    let about = new_session(Arc::clone(&protocol), "/about")
        .start(&test_json!({}))
        .unwrap()
        .boundary
        .unwrap();
    assert_eq!(about.name.as_ref(), "about-ready");

    let mut ordinary = StringWriter::default();
    WebUIHandler::new()
        .render(
            &protocol,
            &test_json!({}),
            &RenderOptions::new("index.html", "/home"),
            &mut ordinary,
        )
        .unwrap();
    assert!(ordinary.output.contains("<p>home</p>"));
    assert!(!ordinary.output.contains("<p>about</p>"));
}

#[test]
fn selected_route_component_can_suspend_inside_generated_host() {
    let protocol = parsed_protocol(
        &document(r#"<route path="/" component="route-page"></route>"#),
        &[(
            "route-page",
            r#"<boundary name="route-body"><p>ready</p></boundary>"#,
        )],
    );
    let mut session = new_session(protocol, "/");
    let start = session.start(&test_json!({})).unwrap();
    assert_eq!(start.boundary.as_ref().unwrap().name.as_ref(), "route-body");
    assert!(String::from_utf8(start.bytes)
        .unwrap()
        .contains(r#"<!--ws:0--><route-page data-ws data-ws-span="0">"#));
}

#[test]
fn selected_route_component_hydration_keys_survive_frozen_state() {
    let entry = document(concat!(
        r#"<route path="/selected" component="selected-page"></route>"#,
        r#"<route path="/inactive" component="inactive-page"></route>"#,
    ));
    let mut protocol = parsed_protocol_data(
        &entry,
        &[
            (
                "selected-page",
                r#"<boundary name="selected-ready"><p>selected</p></boundary>"#,
            ),
            ("inactive-page", "<p>inactive</p>"),
        ],
    );
    protocol.initial_state_strategy = InitialStateStrategy::Components as i32;
    protocol.components.insert(
        "selected-page".to_string(),
        ComponentData {
            hydration_mode: StateProjectionMode::Keys as i32,
            hydration_keys: vec!["selectedHydration".to_string()],
            ..Default::default()
        },
    );
    protocol.components.insert(
        "inactive-page".to_string(),
        ComponentData {
            hydration_mode: StateProjectionMode::Keys as i32,
            hydration_keys: vec!["inactiveHydration".to_string()],
            ..Default::default()
        },
    );

    let mut session = new_session(Arc::new(Protocol::new(protocol)), "/selected");
    let start = session
        .start(&test_json!({
            "selectedHydration": "frozen",
            "inactiveHydration": "do-not-send"
        }))
        .unwrap();
    let boundary = start.boundary.unwrap();
    let committed = session
        .resume(boundary.instance_id, &test_json!({}), BoundaryMode::Final)
        .unwrap();
    let end = session.advance().unwrap();
    assert!(end.done);
    // The route host's own hydration state rides its span-completion record,
    // which the advance step writes after the boundary checkpoint.
    let html = String::from_utf8([committed.bytes, end.bytes].concat()).unwrap();
    assert!(html.contains(r#""state":{"selectedHydration":"frozen"}"#));
    assert!(!html.contains("inactiveHydration"));
}

#[test]
fn route_component_all_projection_keeps_full_state_for_resume() {
    let mut protocol = parsed_protocol_data(
        &document(r#"<route path="/" component="route-page"></route>"#),
        &[(
            "route-page",
            r#"<boundary name="route-ready"><p>ready</p></boundary>"#,
        )],
    );
    protocol.initial_state_strategy = InitialStateStrategy::Components as i32;
    protocol.components.insert(
        "route-page".to_string(),
        ComponentData {
            hydration_mode: StateProjectionMode::All as i32,
            ..Default::default()
        },
    );

    let mut session = new_session(Arc::new(Protocol::new(protocol)), "/");
    let start = session
        .start(&test_json!({ "opaqueRuntimeState": "frozen" }))
        .unwrap();
    let boundary = start.boundary.unwrap();
    let committed = session
        .resume(boundary.instance_id, &test_json!({}), BoundaryMode::Final)
        .unwrap();
    let end = session.advance().unwrap();
    assert!(end.done);
    assert!(String::from_utf8([committed.bytes, end.bytes].concat())
        .unwrap()
        .contains(r#""opaqueRuntimeState":"frozen""#));
}

#[test]
fn route_component_full_strategy_keeps_fast_state_for_resume() {
    let protocol = parsed_protocol(
        &document(r#"<route path="/" component="route-page"></route>"#),
        &[(
            "route-page",
            r#"<boundary name="route-ready"><p>ready</p></boundary>"#,
        )],
    );
    let mut session = new_session(protocol, "/");
    let start = session
        .start(&test_json!({ "fastRuntimeState": "frozen" }))
        .unwrap();
    let boundary = start.boundary.unwrap();
    let committed = session
        .resume(boundary.instance_id, &test_json!({}), BoundaryMode::Final)
        .unwrap();
    assert!(String::from_utf8(committed.bytes)
        .unwrap()
        .contains(r#""fastRuntimeState":"frozen""#));
    assert!(session.advance().unwrap().done);
}

#[test]
fn generated_route_false_if_emits_span_without_boundary_occurrence() {
    let protocol = parsed_protocol(
        &document(r#"<route path="/" component="route-page"></route>"#),
        &[(
            "route-page",
            r#"<if condition="show"><boundary name="route-ready"><p>ready</p></boundary></if>"#,
        )],
    );
    let mut session = new_session(protocol, "/");
    let completed = session.start(&test_json!({ "show": false })).unwrap();
    assert!(completed.done);
    assert!(completed.boundary.is_none());
    let html = String::from_utf8(completed.bytes).unwrap();
    assert!(!html.contains("<!--wb:"));
    assert!(html.contains(r#"<!--ws:0--><route-page data-ws data-ws-span="0">"#));
    assert!(html.contains("</route-page><!--/ws:0-->"));
    assert!(html.contains(r#"<script type="application/json" data-webui-boundary>[2,0,3,0,"#));
    assert!(html.contains("[2,1,4,0,{}]"));
}

#[test]
fn generated_route_without_boundary_or_enclosing_range_is_rejected() {
    let protocol = parsed_protocol(
        &document(r#"<route path="/" component="plain-page"></route>"#),
        &[("plain-page", "<p>plain</p>")],
    );
    let error = new_session(protocol, "/")
        .start(&test_json!({}))
        .unwrap_err();
    assert!(matches!(error, HandlerError::StreamingBoundary(_)));
    assert!(error
        .to_string()
        .contains("component host has no boundary or generated component span"));
}

#[test]
fn nested_component_spans_are_outer_first_and_complete_inner_first() {
    let protocol = parsed_protocol(
        &document("<outer-card></outer-card>"),
        &[
            ("outer-card", "<main><inner-card></inner-card></main>"),
            (
                "inner-card",
                r#"<boundary name="ready"><leaf-card></leaf-card></boundary>"#,
            ),
            ("leaf-card", "<span>leaf</span>"),
        ],
    );
    let mut session = new_session(protocol, "/");
    let start = session.start(&test_json!({})).unwrap();
    let shell = String::from_utf8(start.bytes).unwrap();
    let outer = shell.find("<!--ws:0-->").unwrap();
    let inner = shell.find("<!--ws:1-->").unwrap();
    assert!(outer < inner);

    let boundary = start.boundary.unwrap();
    let committed = session
        .resume(boundary.instance_id, &test_json!({}), BoundaryMode::Final)
        .unwrap();
    let commit = String::from_utf8(committed.bytes).unwrap();
    assert!(commit.contains(r#""enclosingSpanInstanceId":1"#));

    let end = session.advance().unwrap();
    assert!(end.done);
    let html = String::from_utf8(end.bytes).unwrap();
    let inner_end = html.find("<!--/ws:1-->").unwrap();
    let outer_end = html.find("<!--/ws:0-->").unwrap();
    assert!(inner_end < outer_end);
    assert!(html[inner_end..].contains("[2,1,3,1,"));
    assert!(html[outer_end..].contains("[2,2,3,0,"));
}

#[test]
fn update_targets_one_runtime_instance() {
    let protocol = parsed_protocol(
        &document(r#"<boundary name="live"><p>{{count}}</p></boundary>"#),
        &[],
    );
    let mut session = new_session(protocol, "/");
    let boundary = session
        .start(&test_json!({ "count": 1 }))
        .unwrap()
        .boundary
        .unwrap();
    let committed = session
        .resume(
            boundary.instance_id,
            &test_json!({ "count": 1 }),
            BoundaryMode::Updatable,
        )
        .unwrap();
    assert!(!committed.done);
    // An update lands between the commit and the tail: the response is still
    // open, so the host can revise the occurrence it just wrote.
    let update = String::from_utf8(
        session
            .update(boundary.instance_id, &test_json!({ "count": 2 }))
            .unwrap(),
    )
    .unwrap();
    assert!(update.contains("[2,1,2,0,{\"count\":2}]"));

    assert!(session.advance().unwrap().done);
    assert!(session
        .update(boundary.instance_id, &test_json!({ "count": 3 }))
        .unwrap_err()
        .to_string()
        .contains("already completed"));
}

#[test]
fn update_before_terminal_targets_the_runtime_occurrence() {
    let protocol = parsed_protocol(
        &document(concat!(
            r#"<boundary name="live"><p>{{count}}</p></boundary>"#,
            r#"<boundary name="later"><p>later</p></boundary>"#,
        )),
        &[],
    );
    let mut session = new_session(protocol, "/");
    let first = session
        .start(&test_json!({ "count": 1 }))
        .unwrap()
        .boundary
        .unwrap();
    assert!(session
        .resume(
            first.instance_id,
            &test_json!({ "count": 1 }),
            BoundaryMode::Updatable,
        )
        .unwrap()
        .boundary
        .is_none());
    let second = session.advance().unwrap().boundary.unwrap();
    let update = String::from_utf8(
        session
            .update(first.instance_id, &test_json!({ "count": 2 }))
            .unwrap(),
    )
    .unwrap();
    assert!(update.contains("[2,1,2,0,{\"count\":2}]"));
    assert!(session
        .resume(second.instance_id, &test_json!({}), BoundaryMode::Final)
        .unwrap()
        .boundary
        .is_none());
    assert!(session.advance().unwrap().done);
}

#[test]
fn update_rejects_final_and_uncommitted_occurrences_without_poisoning() {
    let protocol = parsed_protocol(
        &document(concat!(
            r#"<boundary name="first"><p>first</p></boundary>"#,
            r#"<boundary name="second"><p>second</p></boundary>"#,
        )),
        &[],
    );
    let mut session = new_session(protocol, "/");
    let first = session.start(&test_json!({})).unwrap().boundary.unwrap();
    assert!(session
        .resume(first.instance_id, &test_json!({}), BoundaryMode::Final)
        .unwrap()
        .boundary
        .is_none());
    let second = session.advance().unwrap().boundary.unwrap();
    assert!(session
        .update(first.instance_id, &test_json!({}))
        .unwrap_err()
        .to_string()
        .contains("committed as final"));
    assert!(session
        .update(second.instance_id, &test_json!({}))
        .unwrap_err()
        .to_string()
        .contains("has not committed"));
    assert!(session
        .resume(second.instance_id, &test_json!({}), BoundaryMode::Final)
        .unwrap()
        .boundary
        .is_none());
    assert!(session.advance().unwrap().done);
}

#[test]
fn wrong_resume_id_is_recoverable_but_render_failure_poisons() {
    let protocol = parsed_protocol(
        &document(r#"<boundary name="ready"><p>ok</p></boundary>"#),
        &[],
    );
    let mut session = new_session(protocol, "/");
    let pending = session.start(&test_json!({})).unwrap().boundary.unwrap();
    assert!(session
        .resume(
            BoundaryInstanceId::from_raw(99),
            &test_json!({}),
            BoundaryMode::Final,
        )
        .unwrap_err()
        .to_string()
        .contains("stale"));
    assert!(session
        .resume(pending.instance_id, &test_json!({}), BoundaryMode::Final)
        .unwrap()
        .boundary
        .is_none());
    assert!(session.advance().unwrap().done);

    let protocol = parsed_protocol(
        &document(
            r#"<boundary name="broken"><for each="item in bad"><p>{{item}}</p></for></boundary>"#,
        ),
        &[],
    );
    let mut broken = new_session(protocol, "/");
    let pending = broken
        .start(&test_json!({ "bad": {} }))
        .unwrap()
        .boundary
        .unwrap();
    assert!(matches!(
        broken.resume(
            pending.instance_id,
            &test_json!({ "bad": {} }),
            BoundaryMode::Final
        ),
        Err(HandlerError::TypeError(_))
    ));
    assert!(broken
        .resume(pending.instance_id, &test_json!({}), BoundaryMode::Final)
        .unwrap_err()
        .to_string()
        .contains("poisoned"));
}

#[test]
fn ordinary_render_transparently_renders_boundary_body() {
    let protocol = parsed_protocol(
        &document(r#"<boundary name="ready"><strong>{{label}}</strong></boundary>"#),
        &[],
    );
    let mut writer = StringWriter::default();
    WebUIHandler::new()
        .render(
            &protocol,
            &test_json!({ "label": "ordinary" }),
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
    // The body is inline between the tape markers, so ordinary rendering must
    // emit it exactly once and skip both markers.
    assert_eq!(
        writer.output.matches("<strong>ordinary</strong>").count(),
        1
    );
    assert!(!writer.output.contains("<!--wb:"));
    assert!(!writer.output.contains("<!--/wb:"));
}

#[test]
fn adjacent_boundaries_do_not_flush_the_same_position_twice() {
    let protocol = parsed_protocol(
        &document(
            r#"<boundary name="first"><p>1</p></boundary><boundary name="second"><p>2</p></boundary>"#,
        ),
        &[],
    );
    let handler = WebUIHandler::new();
    let mut writer = FlushStringWriter::default();
    handler
        .render_streaming(
            &protocol,
            &test_json!({}),
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
    assert!(
        writer
            .flush_positions
            .windows(2)
            .all(|pair| pair[0] < pair[1]),
        "every flush must release bytes the previous flush did not: {:?}",
        writer.flush_positions
    );
    // Shell prefix, one checkpoint per boundary, terminal. The step that
    // returns immediately at the second boundary produced no bytes, so it
    // collapses into the first checkpoint's flush.
    assert_eq!(writer.flushes, 4);
}

#[test]
fn borrowed_api_flushes_each_returned_semantic_step() {
    let protocol = parsed_protocol(
        &document(concat!(
            r#"<boundary name="ready"><p>ok</p></boundary>"#,
            "<footer>tail</footer>",
        )),
        &[],
    );
    let handler = WebUIHandler::new();
    let mut writer = FlushStringWriter::default();
    let mut response = handler
        .stream_response(
            &protocol,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
    let first = response.start(&test_json!({})).unwrap();
    let boundary = first.boundary.unwrap();
    assert!(response
        .resume(boundary.instance_id, &test_json!({}), BoundaryMode::Final)
        .unwrap()
        .boundary
        .is_none());
    assert!(response.advance().unwrap().done);
    drop(response);

    // Prefix, checkpoint, terminal: three flushes, each releasing new bytes,
    // and the checkpoint flush lands exactly where the occurrence's record
    // ends — before any tail byte.
    assert_eq!(writer.flushes, 3);
    assert!(writer.flush_positions.windows(2).all(|p| p[0] < p[1]));
    let checkpoint = writer.flush_positions[1];
    assert!(writer.output[..checkpoint].ends_with("<webui-hydrate></webui-hydrate>"));
    assert!(!writer.output[..checkpoint].contains("<footer>tail</footer>"));
    assert!(writer.output[checkpoint..].starts_with("<footer>tail</footer>"));
    assert!(writer.ended);
}

#[test]
fn borrowed_and_owned_sessions_produce_identical_step_segments() {
    let entry = document(concat!(
        r#"<boundary name="first"><p>1</p></boundary>"#,
        "<hr>",
        r#"<boundary name="second"><p>2</p></boundary>"#,
        "<footer>tail</footer>",
    ));
    let protocol = parsed_protocol(&entry, &[]);
    let handler = WebUIHandler::new();

    let mut owned = new_session(Arc::clone(&protocol), "/");
    let mut segments: Vec<String> = Vec::new();
    let mut step = owned.start(&test_json!({})).unwrap();
    segments.push(String::from_utf8(step.bytes.clone()).unwrap());
    while !step.done {
        step = match step.boundary.as_ref() {
            Some(boundary) => owned
                .resume(boundary.instance_id, &test_json!({}), BoundaryMode::Final)
                .unwrap(),
            None => owned.advance().unwrap(),
        };
        segments.push(String::from_utf8(step.bytes.clone()).unwrap());
    }

    let mut writer = FlushStringWriter::default();
    let mut response = handler
        .stream_response(
            &protocol,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
    let mut status = response.start(&test_json!({})).unwrap();
    while !status.done {
        status = match status.boundary.as_ref() {
            Some(boundary) => response
                .resume(boundary.instance_id, &test_json!({}), BoundaryMode::Final)
                .unwrap(),
            None => response.advance().unwrap(),
        };
    }
    drop(response);

    assert_eq!(
        segments.concat(),
        writer.output,
        "owned steps must reassemble into the borrowed byte stream"
    );
    // Every non-empty owned step is one flushed segment, so the borrowed
    // writer's flush positions are exactly the owned step offsets.
    let mut offsets = Vec::new();
    let mut position = 0usize;
    for segment in &segments {
        position += segment.len();
        if !segment.is_empty() {
            offsets.push(position);
        }
    }
    assert_eq!(offsets, writer.flush_positions);
}

#[test]
fn borrowed_disconnect_poisons_the_response() {
    let protocol = parsed_protocol(
        &document(r#"<boundary name="ready"><p>ok</p></boundary>"#),
        &[],
    );
    let handler = WebUIHandler::new();
    let mut writer = DisconnectWriter;
    let mut response = handler
        .stream_response(
            &protocol,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
    assert!(matches!(
        response.start(&test_json!({})),
        Err(HandlerError::ClientDisconnected)
    ));
    assert!(response
        .start(&test_json!({}))
        .unwrap_err()
        .to_string()
        .contains("poisoned"));
}

#[test]
fn fixed_entry_plan_and_name_table_are_absent() {
    let route_handler = include_str!("../src/route_handler.rs");
    let streaming_module = include_str!("../src/streaming/mod.rs");
    assert!(!route_handler.contains("streaming_boundaries"));
    assert!(!route_handler.contains("StreamingEntryPlan"));
    assert!(!streaming_module.contains("StreamingEntryPlan"));
}

#[derive(Default)]
struct StringWriter {
    output: String,
}

impl ResponseWriter for StringWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.output.push_str(content);
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct FlushStringWriter {
    output: String,
    flushes: usize,
    flush_positions: Vec<usize>,
    ended: bool,
}

impl ResponseWriter for FlushStringWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.output.push_str(content);
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        self.ended = true;
        Ok(())
    }
}

impl FlushWriter for FlushStringWriter {
    fn flush(&mut self) -> webui_handler::Result<()> {
        self.flushes += 1;
        self.flush_positions.push(self.output.len());
        Ok(())
    }
}

struct DisconnectWriter;

impl ResponseWriter for DisconnectWriter {
    fn write(&mut self, _content: &str) -> webui_handler::Result<()> {
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

impl FlushWriter for DisconnectWriter {
    fn flush(&mut self) -> webui_handler::Result<()> {
        Err(HandlerError::ClientDisconnected)
    }
}
