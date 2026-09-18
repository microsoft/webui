// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{
    document, handler, ordinary, outlet_protocol, parsed_protocol, routes, FailingSelectedWriter,
    FlushStringWriter, CHILDREN,
};
use std::sync::Arc;
use webui_handler::{
    BoundaryMode, HandlerError, Protocol, RenderOptions, SessionOptions, StreamingSession,
    WebUIHandler,
};
use webui_test_utils::test_json;

fn marker_protocol(children: &str, shell: &str) -> Arc<Protocol> {
    // A tail checkpoint gives streamed route hosts a valid enclosing span.
    let shell = format!("{shell}<boundary name=\"tail\"><footer>checkpoint</footer></boundary>");
    outlet_protocol(children, &shell, "<p>B</p>")
}

fn render_with(handler: &WebUIHandler, protocol: &Protocol, streamed: bool) -> String {
    let mut writer = FlushStringWriter::default();
    let state = test_json!({});
    let options = RenderOptions::new("index.html", "/b");
    if streamed {
        handler
            .render_streaming(protocol, &state, &options, &mut writer)
            .unwrap();
    } else {
        handler
            .render(protocol, &state, &options, &mut writer)
            .unwrap();
    }
    writer.output
}

fn between<'a>(output: &'a str, before: &str, after: &str) -> &'a str {
    output
        .split_once(before)
        .unwrap()
        .1
        .split_once(after)
        .unwrap()
        .0
}

#[test]
fn empty_and_consumed_outlets_keep_paired_markers_in_both_render_modes() {
    for children in ["", CHILDREN] {
        let protocol = marker_protocol(
            children,
            "<main>first-before<outlet />first-after<hr>second-before<outlet />second-after</main>",
        );
        for streamed in [false, true] {
            let output = render_with(&handler(), &protocol, streamed);
            let first = between(&output, "first-before", "first-after");
            let second = between(&output, "second-before", "second-after");
            assert!(first.starts_with("<!--wo-->"), "{output}");
            assert!(first.ends_with("<!--/wo-->"), "{output}");
            assert_eq!(second, "<!--wo--><!--/wo-->");
            assert_eq!(second.len(), 19);
            if children.is_empty() {
                assert_eq!(first, second);
            } else {
                let expected = if streamed {
                    [
                        ("a", "outlet-a", false),
                        ("b", "outlet-b", true),
                        ("c", "outlet-c", false),
                    ]
                } else {
                    [
                        ("b", "outlet-b", true),
                        ("a", "outlet-a", false),
                        ("c", "outlet-c", false),
                    ]
                };
                assert_eq!(routes(first), expected, "{output}");
            }
            assert_eq!(output.matches("<!--wo-->").count(), 2, "{output}");
            assert_eq!(output.matches("<!--/wo-->").count(), 2, "{output}");
        }
    }
}

#[test]
fn unmatched_outlet_marks_the_full_hidden_sibling_range() {
    let protocol = outlet_protocol(CHILDREN, "before<outlet />after", "<p>B</p>");
    let output = ordinary(&protocol, "/missing");
    let range = between(&output, "before", "after");
    assert!(range.starts_with("<!--wo-->"), "{output}");
    assert!(range.ends_with("<!--/wo-->"), "{output}");
    assert_eq!(
        routes(range),
        [
            ("a", "outlet-a", false),
            ("b", "outlet-b", false),
            ("c", "outlet-c", false),
        ]
    );
}

#[test]
fn empty_outlet_closes_at_the_existing_continuation_limit() {
    // The two route hosts and their enclosing outlet retain nine frames.
    // Each conditional uses two; the empty outlet caller uses the last one.
    let depth = (webui_handler::MAX_CONTINUATION_DEPTH - 10) / 2;
    for extra in [0, 1] {
        let shell = format!(
            "{}<outlet />{}<boundary name=\"tail\"><footer>checkpoint</footer></boundary>",
            "<if condition=\"yes\">".repeat(depth + extra),
            "</if>".repeat(depth + extra),
        );
        let mut session = StreamingSession::new(
            Arc::new(handler()),
            outlet_protocol(
                CHILDREN,
                concat!(
                    "<outlet />",
                    r#"<boundary name="shell-tail"><footer>shell-checkpoint</footer></boundary>"#,
                ),
                &shell,
            ),
            SessionOptions::new("index.html", "/b"),
        )
        .unwrap();
        let result = session.start(test_json!({ "yes": true }));
        if extra == 0 {
            let mut step = result.unwrap();
            let mut output = String::new();
            for (name, owner, closed) in
                [("tail", "outlet-b", 1), ("shell-tail", "outlet-shell", 2)]
            {
                assert!(!step.done);
                let boundary = step.boundary.unwrap();
                assert_eq!(boundary.name.as_ref(), name);
                assert_eq!(boundary.owner.as_ref(), owner);
                output.push_str(&String::from_utf8(step.bytes).unwrap());
                assert_eq!(output.matches("<!--wo--><!--/wo-->").count(), 1, "{output}");
                assert_eq!(output.matches("<!--wo-->").count(), 2, "{output}");
                assert_eq!(output.matches("<!--/wo-->").count(), closed, "{output}");
                let commit = session
                    .resume_current(boundary.instance_id, BoundaryMode::Final)
                    .unwrap();
                assert!(!commit.done && commit.boundary.is_none());
                output.push_str(&String::from_utf8(commit.bytes).unwrap());
                assert_eq!(output.matches("<!--/wo-->").count(), closed, "{output}");
                step = session.advance().unwrap();
            }
            assert!(step.done && step.boundary.is_none());
            output.push_str(&String::from_utf8(step.bytes).unwrap());
            assert_eq!(output.matches("<!--wo-->").count(), 2, "{output}");
            assert_eq!(output.matches("<!--/wo-->").count(), 2, "{output}");
            assert_eq!(
                output.matches("<footer>checkpoint</footer>").count(),
                1,
                "{output}"
            );
            assert_eq!(
                output.matches("<footer>shell-checkpoint</footer>").count(),
                1,
                "{output}"
            );
        } else {
            let Err(error) = result else {
                panic!("one more conditional must exceed the cap");
            };
            assert!(error.to_string().contains(&format!(
                "continuation depth exceeds {}",
                webui_handler::MAX_CONTINUATION_DEPTH
            )));
        }
    }
}

fn nested_protocol() -> Arc<Protocol> {
    parsed_protocol(
        &document(concat!(
            r#"<route path="/" component="outlet-shell">"#,
            r#"<route path="b" component="outlet-b">"#,
            r#"<route path="c" component="outlet-c" exact />"#,
            r#"<route path="other" component="outlet-a" exact />"#,
            "</route>",
            r#"<route path="a" component="outlet-a" exact />"#,
            "</route>",
        )),
        &[
            (
                "outlet-shell",
                concat!(
                    "<main>outer-before<outlet />outer-after</main>",
                    r#"<boundary name="shell-tail"><footer>shell-tail</footer></boundary>"#,
                ),
            ),
            (
                "outlet-b",
                concat!(
                    "<section>inner-before<outlet />inner-after</section>",
                    r#"<boundary name="branch-tail"><footer>B-tail</footer></boundary>"#,
                ),
            ),
            (
                "outlet-c",
                r#"<boundary name="leaf"><p>C</p></boundary><footer>C-tail</footer>"#,
            ),
            ("outlet-a", "<p>A</p>"),
        ],
    )
}

fn assert_nested_ranges(output: &str) {
    let outer = between(output, "outer-before", "outer-after");
    let inner = between(output, "inner-before", "inner-after");
    assert!(
        outer.starts_with("<!--wo-->") && outer.ends_with("<!--/wo-->"),
        "{output}"
    );
    assert!(
        inner.starts_with("<!--wo-->") && inner.ends_with("<!--/wo-->"),
        "{output}"
    );
    assert_eq!(outer.matches("<!--wo-->").count(), 2, "{output}");
    assert_eq!(outer.matches("<!--/wo-->").count(), 2, "{output}");
    assert_eq!(inner.matches("<!--wo-->").count(), 1, "{output}");
    assert_eq!(inner.matches("<!--/wo-->").count(), 1, "{output}");
    assert_eq!(
        routes(inner),
        [("c", "outlet-c", true), ("other", "outlet-a", false)],
        "{output}"
    );
}

#[test]
fn nested_outlets_bracket_their_own_ordinary_expansions() {
    assert_nested_ranges(&ordinary(&nested_protocol(), "/b/c"));
}

#[test]
fn nested_outlet_markers_stay_open_across_streamed_resume() {
    let mut session = StreamingSession::new(
        Arc::new(handler()),
        nested_protocol(),
        SessionOptions::new("index.html", "/b/c"),
    )
    .unwrap();
    let mut step = session.start(test_json!({})).unwrap();
    let mut output = String::new();
    for (name, owner, closed) in [
        ("leaf", "outlet-c", 0),
        ("branch-tail", "outlet-b", 1),
        ("shell-tail", "outlet-shell", 2),
    ] {
        assert!(!step.done);
        let boundary = step.boundary.unwrap();
        assert_eq!(boundary.name.as_ref(), name);
        assert_eq!(boundary.owner.as_ref(), owner);
        output.push_str(&String::from_utf8(step.bytes).unwrap());
        assert_eq!(output.matches("<!--wo-->").count(), 2, "{output}");
        assert_eq!(output.matches("<!--/wo-->").count(), closed, "{output}");
        let commit = session
            .resume_current(boundary.instance_id, BoundaryMode::Final)
            .unwrap();
        assert!(!commit.done && commit.boundary.is_none());
        output.push_str(&String::from_utf8(commit.bytes).unwrap());
        assert_eq!(output.matches("<!--/wo-->").count(), closed, "{output}");
        step = session.advance().unwrap();
    }
    assert!(step.done && step.boundary.is_none());
    output.push_str(&String::from_utf8(step.bytes).unwrap());
    assert_nested_ranges(&output);
    assert_eq!(
        output.matches("<footer>C-tail</footer>").count(),
        1,
        "{output}"
    );
    assert_eq!(
        output.matches("<footer>B-tail</footer>").count(),
        1,
        "{output}"
    );
    assert_eq!(
        output.matches("<footer>shell-tail</footer>").count(),
        1,
        "{output}"
    );
}

#[test]
fn outlet_marker_transport_errors_stop_both_render_modes() {
    for children in ["", CHILDREN] {
        let protocol = marker_protocol(children, "before<outlet />after");
        for marker in ["<!--wo-->", "<!--/wo-->"] {
            for streamed in [false, true] {
                let mut writer = FailingSelectedWriter {
                    fail_on: Some(marker),
                    ..Default::default()
                };
                let renderer = handler();
                let state = test_json!({});
                let options = RenderOptions::new("index.html", "/b");
                let result = if streamed {
                    renderer.render_streaming(&protocol, &state, &options, &mut writer)
                } else {
                    renderer.render(&protocol, &state, &options, &mut writer)
                };
                assert!(matches!(result, Err(HandlerError::ClientDisconnected)));
                assert!(writer.failed);
                assert_eq!(writer.writes_after_failure, 0);
                assert!(!writer.output.contains("after"), "{}", writer.output);
                if marker == "<!--wo-->" {
                    assert_eq!(routes(&writer.output).len(), 1, "{}", writer.output);
                } else {
                    assert!(writer.output.contains("<!--wo-->"), "{}", writer.output);
                    assert_eq!(
                        routes(&writer.output).len(),
                        if children.is_empty() { 1 } else { 4 },
                        "{}",
                        writer.output
                    );
                }
            }
        }
    }
}

#[test]
fn fast_and_plugin_free_outlet_ranges_remain_marker_free() {
    let renderers = [
        WebUIHandler::with_plugin(|| {
            Box::new(webui_handler::plugin::fast_v2::FastV2HydrationPlugin::new())
        }),
        WebUIHandler::with_plugin(|| {
            Box::new(webui_handler::plugin::fast_v3::FastV3HydrationPlugin::new())
        }),
    ];
    for children in ["", CHILDREN] {
        let protocol = marker_protocol(children, "before<outlet />after");
        for streamed in [false, true] {
            let plain = render_with(&WebUIHandler::new(), &protocol, streamed);
            assert!(!plain.contains("<!--wo-->") && !plain.contains("<!--/wo-->"));
            let expected = between(&plain, "before", "after");
            for renderer in &renderers {
                let output = render_with(renderer, &protocol, streamed);
                assert!(!output.contains("<!--wo-->") && !output.contains("<!--/wo-->"));
                assert_eq!(between(&output, "before", "after"), expected, "{output}");
            }
            let webui = render_with(&handler(), &protocol, streamed);
            assert_eq!(
                between(&webui, "before", "after"),
                format!("<!--wo-->{expected}<!--/wo-->"),
                "{webui}"
            );
        }
    }
}
