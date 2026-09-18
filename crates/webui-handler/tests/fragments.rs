// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use webui_handler::{
    BoundaryMode, FlushWriter, HandlerError, Protocol, RenderOptions, ResponseWriter,
    SessionOptions, StreamingSession, WebUIHandler, MAX_FRAGMENT_CALL_DEPTH,
    MAX_FRAGMENT_INVOCATIONS,
};
use webui_protocol::{ConditionExpr, FragmentList, WebUIFragment as F, WebUIProtocol};
use webui_test_utils::test_json;

#[derive(Default)]
struct Writer(String);

impl ResponseWriter for Writer {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.0.push_str(content);
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

impl FlushWriter for Writer {
    fn flush(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

fn protocol(records: impl IntoIterator<Item = (&'static str, Vec<F>)>) -> Protocol {
    Protocol::new(WebUIProtocol::new(
        records
            .into_iter()
            .map(|(id, fragments)| {
                (
                    id.to_owned(),
                    FragmentList {
                        fragments,
                        contains_boundary: true,
                    },
                )
            })
            .collect(),
    ))
}

fn render(protocol: &Protocol, state: &Value) -> webui_handler::Result<String> {
    let mut writer = Writer::default();
    WebUIHandler::new().render(
        protocol,
        state,
        &RenderOptions::new("entry", "/"),
        &mut writer,
    )?;
    Ok(writer.0)
}

#[test]
fn fused_repeat_records_restore_monikers_and_stop_at_writer_failure() {
    struct FailAtStop(String);
    impl ResponseWriter for FailAtStop {
        fn write(&mut self, content: &str) -> webui_handler::Result<()> {
            if content == "stop" {
                return Err(HandlerError::ClientDisconnected);
            }
            self.0.push_str(content);
            Ok(())
        }
        fn end(&mut self) -> webui_handler::Result<()> {
            panic!("a failed render must not finalize the writer")
        }
    }
    let protocol = protocol([
        (
            "entry",
            vec![F::for_loop("row", "rows", "row"), F::raw("AFTER")],
        ),
        (
            "row",
            vec![
                F::signal("row.name", false),
                F::for_loop("row", "row.children", "child"),
                F::signal("row.name", false),
            ],
        ),
        ("child", vec![F::signal("row.name", false)]),
    ]);
    let state = test_json!({"rows": [
        {"name": "A", "children": [{"name": "B"}, {"name": "stop"}, {"name": "INNER"}]},
        {"name": "OUTER", "children": []}
    ]});
    assert_eq!(
        render(&protocol, &state).unwrap(),
        "ABstopINNERAOUTEROUTERAFTER"
    );
    let mut writer = FailAtStop(String::new());
    assert!(matches!(
        WebUIHandler::new().render(
            &protocol,
            &state,
            &RenderOptions::new("entry", "/"),
            &mut writer,
        ),
        Err(HandlerError::ClientDisconnected)
    ));
    assert_eq!(writer.0, "AB");
}

#[test]
fn empty_component_scopes_restore_nested_props_and_caller_state() {
    let prop = F {
        fragment: Some(webui_protocol::web_ui_fragment::Fragment::Attribute(
            webui_protocol::WebUIFragmentAttribute {
                name: "name".into(),
                value: "row.name".into(),
                complex: true,
                attr_start: true,
                ..Default::default()
            },
        )),
    };
    let protocol = protocol([
        (
            "entry",
            vec![
                F::component("empty"),
                F::raw("|"),
                F::signal("name", false),
                F::raw("|"),
                F::component("empty"),
            ],
        ),
        (
            "empty",
            vec![F::for_loop("row", "rows", "item"), F::signal("name", false)],
        ),
        (
            "item",
            vec![prop, F::component("child"), F::signal("row.name", false)],
        ),
        ("child", vec![F::signal("name", false)]),
    ]);
    let state = test_json!({"name": "O", "rows": [{"name": "A"}, {"name": "B"}]});
    assert_eq!(render(&protocol, &state).unwrap(), "AABBO|O|AABBO");
}

#[test]
fn plugin_free_conditions_use_only_live_continuations() {
    let depth = webui_handler::MAX_CONTINUATION_DEPTH / 2 + 1;
    let mut records = HashMap::new();
    records.insert(
        "entry".to_owned(),
        FragmentList {
            fragments: vec![
                F::signal("}}}webui:head_start", true),
                F::if_cond(ConditionExpr::identifier("yes"), "level-0"),
                F::signal("}}}webui:body_end", true),
            ],
            contains_boundary: false,
        },
    );
    for index in 0..depth {
        records.insert(
            format!("level-{index}"),
            FragmentList {
                fragments: if index + 1 == depth {
                    vec![
                        F::if_cond(
                            ConditionExpr::identifier("no"),
                            "unreachable-missing-record",
                        ),
                        F::raw("deep-tail"),
                    ]
                } else {
                    vec![F::if_cond(
                        ConditionExpr::identifier("yes"),
                        format!("level-{}", index + 1),
                    )]
                },
                contains_boundary: false,
            },
        );
    }
    // State planning validates references even when their condition is false.
    records.insert(
        "unreachable-missing-record".to_owned(),
        FragmentList::default(),
    );
    let protocol = Protocol::new(WebUIProtocol::new(records));
    let state = test_json!({"yes": true, "no": false});
    assert_eq!(render(&protocol, &state).unwrap(), "deep-tail");
    let handler = WebUIHandler::new();
    let options = RenderOptions::new("entry", "/");
    let mut writer = Writer::default();
    let mut response = handler
        .stream_response(&protocol, &options, &mut writer)
        .unwrap();
    assert!(response.start(&state).unwrap().done);
    drop(response);
    assert!(writer.0.contains("deep-tail"));
}

fn card_protocol(parser: webui_parser::HtmlParser) -> Protocol {
    Protocol::new(card_document(parser))
}

fn card_document(mut parser: webui_parser::HtmlParser) -> WebUIProtocol {
    let artifacts = parser.take_plugin_artifacts().unwrap();
    let mut document = WebUIProtocol::new(parser.into_fragment_records());
    document.components.insert(
        "my-card".into(),
        webui_protocol::ComponentData {
            uses_shadow_dom: true,
            ..Default::default()
        },
    );
    if let webui_parser::plugin::ParserPluginArtifacts::ComponentTemplates(templates) = artifacts {
        for template in templates {
            document.components.insert(
                template.tag_name,
                webui_protocol::ComponentData {
                    template: template.template,
                    template_json: template.template_json,
                    template_functions: template.template_functions,
                    uses_shadow_dom: template.uses_shadow_dom,
                    ..Default::default()
                },
            );
        }
    }
    document.populate_style_closures(&["entry"]);
    document
}

fn streaming_card(template: &str) -> Protocol {
    let mut parser = webui_parser::HtmlParser::new();
    parser
        .component_registry_mut()
        .register_component(webui_parser::ComponentRegistration::new(
            "my-card", template, None, true,
        ))
        .unwrap();
    parser
        .parse(
            "entry",
            "<html><head></head><body><my-card></my-card></body></html>",
        )
        .unwrap();
    card_protocol(parser)
}

fn stream_records(html: &str) -> impl Iterator<Item = Value> + '_ {
    html.split("<script type=\"application/json\" data-webui-boundary>")
        .skip(1)
        .map(|script| serde_json::from_str(script.split("</script>").next().unwrap()).unwrap())
}

fn recursive_protocol() -> Protocol {
    protocol([
        ("entry", vec![F::render("node", "tree", "node")]),
        (
            "node",
            vec![
                F::signal("node.label", false),
                F::if_cond(ConditionExpr::identifier("node.children.length"), "branch"),
            ],
        ),
        (
            "branch",
            vec![F::for_loop("child", "node.children", "item")],
        ),
        ("item", vec![F::render("node", "child", "node")]),
    ])
}

fn chain(depth: usize) -> Value {
    let mut node = test_json!({"label": "x", "children": []});
    for _ in 1..depth {
        node = test_json!({"label": "x", "children": [node]});
    }
    test_json!({"tree": node})
}

#[test]
fn exact_call_depth_with_if_and_repeat_frames_is_allowed() {
    assert_eq!(
        render(&recursive_protocol(), &chain(MAX_FRAGMENT_CALL_DEPTH)).unwrap(),
        "x".repeat(MAX_FRAGMENT_CALL_DEPTH),
    );
}

#[test]
fn call_depth_plus_one_is_a_typed_error() {
    assert!(matches!(
        render(&recursive_protocol(), &chain(MAX_FRAGMENT_CALL_DEPTH + 1)),
        Err(HandlerError::FragmentCallDepth {
            limit: MAX_FRAGMENT_CALL_DEPTH
        }),
    ));
}

#[test]
fn unconditional_recursion_fails_without_native_recursion() {
    let protocol = protocol([
        ("entry", vec![F::render("self", "", "")]),
        ("self", vec![F::render("self", "", "")]),
    ]);
    assert!(matches!(
        render(&protocol, &Value::Null),
        Err(HandlerError::FragmentCallDepth { .. })
    ));
}

fn invocation_protocol() -> Protocol {
    protocol([
        ("entry", vec![F::for_loop("item", "items", "item")]),
        ("item", vec![F::render("leaf", "", "")]),
        ("leaf", vec![F::raw("x")]),
    ])
}

#[test]
fn exact_invocation_budget_and_plus_one() {
    for count in [MAX_FRAGMENT_INVOCATIONS, MAX_FRAGMENT_INVOCATIONS + 1] {
        let state = test_json!({"items": vec![Value::Null; count]});
        let result = render(&invocation_protocol(), &state);
        if count == MAX_FRAGMENT_INVOCATIONS {
            assert_eq!(result.unwrap().len(), count);
        } else {
            assert!(matches!(
                result,
                Err(HandlerError::FragmentCallBudget {
                    limit: MAX_FRAGMENT_INVOCATIONS
                })
            ));
        }
    }
}

#[test]
fn aliases_hide_caller_loops_restore_and_own_missing_members() {
    let protocol = protocol([
        ("entry", vec![F::for_loop("caller", "items", "caller")]),
        (
            "caller",
            vec![
                F::render("scoped", "caller", "row"),
                F::raw("|"),
                F::signal("caller.name", false),
            ],
        ),
        (
            "scoped",
            vec![
                F::signal("row.name", false),
                F::raw("/"),
                F::signal("row.missing", false),
                F::raw("/"),
                F::signal("caller.name", false),
                F::raw("/"),
                F::render("plain", "", ""),
                F::raw("/"),
                F::signal("row.name", false),
            ],
        ),
        ("plain", vec![F::signal("row.name", false)]),
    ]);
    assert_eq!(
        render(
            &protocol,
            &test_json!({
                "items": [{"name":"captured"}],
                "caller": {"name":"owner"},
                "row": {"name":"root", "missing":"must-not-leak"}
            })
        )
        .unwrap(),
        "captured//owner/root/captured|captured"
    );
}

#[test]
fn loop_variables_shadow_aliases_and_restore_them() {
    let protocol = protocol([
        ("entry", vec![F::render("body", "source", "row")]),
        (
            "body",
            vec![
                F::for_loop("row", "rows", "item"),
                F::signal("row.name", false),
            ],
        ),
        ("item", vec![F::signal("row.name", false)]),
    ]);
    assert_eq!(
        render(
            &protocol,
            &test_json!({
                "source":{"name":"outer"}, "rows":[{"name":"inner"}]
            })
        )
        .unwrap(),
        "innerouter"
    );
}

#[test]
fn explicit_falsy_inputs_are_valid_but_missing_input_is_not() {
    let protocol = protocol([
        ("entry", vec![F::render("body", "value", "arg")]),
        ("body", vec![F::raw("called")]),
    ]);
    for value in [
        Value::Null,
        Value::Bool(false),
        Value::from(0),
        Value::from(""),
        test_json!({}),
        test_json!([]),
    ] {
        assert_eq!(
            render(&protocol, &test_json!({"value":value})).unwrap(),
            "called"
        );
    }
    assert!(matches!(
        render(&protocol, &test_json!({})),
        Err(HandlerError::FragmentScopeMissing(_))
    ));
}

#[test]
fn aliased_missing_scope_does_not_fall_back_to_owner_root() {
    let protocol = protocol([
        ("entry", vec![F::render("body", "source", "row")]),
        ("body", vec![F::render("leaf", "row.missing", "child")]),
        ("leaf", vec![F::raw("bad")]),
    ]);
    assert!(matches!(
        render(
            &protocol,
            &test_json!({
                "source": {}, "row":{"missing":"must-not-leak"}
            })
        ),
        Err(HandlerError::FragmentScopeMissing(_))
    ));
}

#[test]
fn shared_body_records_are_replayed_not_globally_visited() {
    let protocol = protocol([
        (
            "entry",
            vec![
                F::render("leaf", "one", "row"),
                F::render("leaf", "two", "row"),
            ],
        ),
        ("leaf", vec![F::signal("row", false)]),
    ]);
    assert_eq!(
        render(&protocol, &test_json!({"one":"a","two":"b"})).unwrap(),
        "ab"
    );
}

#[test]
fn synthetic_length_is_valid_only_at_the_end_of_scope_path() {
    let protocol = protocol([
        ("entry", vec![F::render("body", "items.length", "n")]),
        ("body", vec![F::signal("n", false)]),
    ]);
    assert_eq!(
        render(&protocol, &test_json!({"items":[1,2]})).unwrap(),
        "2"
    );
    for (text, expected) in [("é", "2"), ("😀", "4"), ("é😀", "6")] {
        assert_eq!(
            render(&protocol, &test_json!({"items":text})).unwrap(),
            expected
        );
    }
}

#[test]
fn webui_markers_wrap_every_invocation_and_plain_ssr_has_none() {
    let protocol = protocol([
        (
            "entry",
            vec![F::render("leaf", "", ""), F::render("leaf", "", "")],
        ),
        ("leaf", vec![F::raw("x")]),
    ]);
    let mut writer = Writer::default();
    WebUIHandler::with_plugin(|| {
        Box::new(webui_handler::plugin::webui::WebUIHydrationPlugin::new())
    })
    .render(
        &protocol,
        &Value::Null,
        &RenderOptions::new("entry", "/"),
        &mut writer,
    )
    .unwrap();
    assert_eq!(writer.0, "<!--wf-->x<!--/wf--><!--wf-->x<!--/wf-->");
    assert_eq!(render(&protocol, &Value::Null).unwrap(), "xx");
}

#[test]
fn ordinary_scoped_calls_render_borrowed_inputs_without_capturing_sources() {
    let protocol = protocol([
        ("entry", vec![F::render("leaf", "source", "row")]),
        ("leaf", vec![F::signal("row.label", false)]),
    ]);
    let mut writer = Writer::default();
    WebUIHandler::with_plugin(|| {
        Box::new(webui_handler::plugin::webui::WebUIHydrationPlugin::new())
    })
    .render(
        &protocol,
        &test_json!({"source": {"label": "OLD"}}),
        &RenderOptions::new("entry", "/"),
        &mut writer,
    )
    .unwrap();
    // Nothing can adopt an ordinary response, so the scope is borrowed and no
    // owning projection is ever recorded.
    assert!(writer.0.contains("OLD"), "{}", writer.0);
    assert!(writer.0.contains("<!--wf-->"), "{}", writer.0);
    assert!(!writer.0.contains("<!--wf:"), "{}", writer.0);
    assert!(!writer.0.contains("fragmentSource"), "{}", writer.0);
}

fn streaming_protocol() -> Protocol {
    protocol([
        (
            "entry",
            vec![
                F::raw("<html><head>"),
                F::signal("}}}webui:head_start", true),
                F::raw("</head><body>"),
                F::signal("}}}webui:body_start", true),
                F::render("body", "source", "item"),
                F::raw("<aside>"),
                F::signal("source.label", false),
                F::raw("</aside>"),
                F::signal("}}}webui:body_end", true),
                F::raw("</body></html>"),
            ],
        ),
        (
            "body",
            vec![
                F::boundary(0, "entry", "ready", None),
                F::raw("<p>"),
                F::signal("item.label", false),
                F::raw("/"),
                F::signal("owner", false),
                F::raw("</p>"),
                F::boundary_end(0),
                F::raw("<footer>"),
                F::signal("item.label", false),
                F::raw("/"),
                F::signal("owner", false),
                F::raw("</footer>"),
            ],
        ),
    ])
}

#[test]
fn streaming_alias_survives_root_replacement_and_parent_tail() {
    let mut session = StreamingSession::new(
        Arc::new(WebUIHandler::new()),
        Arc::new(streaming_protocol()),
        SessionOptions::new("entry", "/"),
    )
    .unwrap();
    let start = session
        .start(test_json!({"source":{"label":"captured"},"owner":"before"}))
        .unwrap();
    let boundary = start.boundary.unwrap();
    let commit = session
        .resume(
            boundary.instance_id,
            test_json!({"source":{"label":"replacement"},"owner":"after"}),
            BoundaryMode::Final,
        )
        .unwrap();
    let commit = String::from_utf8(commit.bytes).unwrap();
    assert!(commit.contains("<p>captured/after</p>"));
    assert!(!commit.contains("<footer>"));
    let tail = session.advance().unwrap();
    assert!(tail.done);
    assert!(String::from_utf8(tail.bytes)
        .unwrap()
        .contains("<footer>captured/after</footer><aside>replacement</aside>"));
}

#[test]
fn writer_failure_is_returned_before_more_fragment_work() {
    struct Fail;
    impl ResponseWriter for Fail {
        fn write(&mut self, _: &str) -> webui_handler::Result<()> {
            Err(HandlerError::ClientDisconnected)
        }

        fn end(&mut self) -> webui_handler::Result<()> {
            Ok(())
        }
    }
    let protocol = protocol([
        ("entry", vec![F::render("body", "", "")]),
        ("body", vec![F::raw("fail"), F::render("body", "", "")]),
    ]);
    assert!(matches!(
        WebUIHandler::new().render(
            &protocol,
            &Value::Null,
            &RenderOptions::new("entry", "/"),
            &mut Fail,
        ),
        Err(HandlerError::ClientDisconnected)
    ));
}

#[test]
fn component_props_remain_visible_inside_fragment_body() {
    let mut parser = webui_parser::HtmlParser::new();
    parser
        .component_registry_mut()
        .register_component(webui_parser::ComponentRegistration::new(
            "my-card",
            r#"<fragment name="body">{{label}}</fragment><render fragment="body" />"#,
            None,
            true,
        ))
        .unwrap();
    parser
        .parse("entry", r#"<my-card label="local"></my-card>"#)
        .unwrap();
    let protocol = card_protocol(parser);
    assert!(render(&protocol, &test_json!({"label":"global"}))
        .unwrap()
        .contains("local"));
}

#[test]
fn synthetic_input_lengths_use_caller_loops_and_borrowed_props_before_owner_state() {
    let loop_protocol = protocol([
        ("entry", vec![F::for_loop("item", "items", "caller")]),
        ("caller", vec![F::render("length", "item.length", "n")]),
        ("length", vec![F::signal("n", false)]),
    ]);
    assert_eq!(
        render(
            &loop_protocol,
            &test_json!({
                "items":["é","😀"], "item":{"length":777}
            })
        )
        .unwrap(),
        "24"
    );

    let mut parser = webui_parser::HtmlParser::new();
    parser
        .component_registry_mut()
        .register_component(webui_parser::ComponentRegistration::new(
            "my-card",
            concat!(
                r#"<fragment name="length">{{n}}</fragment>"#,
                r#"<render fragment="length" scope="{{label.length}}" as="n" />"#,
            ),
            None,
            true,
        ))
        .unwrap();
    parser
        .parse("entry", r#"<my-card :label="{{text}}"></my-card>"#)
        .unwrap();
    let protocol = card_protocol(parser);
    let output = render(
        &protocol,
        &test_json!({
            "text":"é", "label":{"length":777}
        }),
    )
    .unwrap();
    assert!(output.contains(">2<"), "{output}");
    assert!(!output.contains("777"), "{output}");
}

#[test]
fn caller_loop_does_not_remove_the_callee_owning_component_prop() {
    let mut parser = webui_parser::HtmlParser::new();
    parser
        .component_registry_mut()
        .register_component(webui_parser::ComponentRegistration::new(
            "my-card",
            concat!(
                r#"<fragment name="body">{{label}}</fragment>"#,
                r#"<for each="label in labels"><render fragment="body" />{{label}}</for>"#,
                "{{label}}",
            ),
            None,
            true,
        ))
        .unwrap();
    parser
        .parse("entry", r#"<my-card label="prop"></my-card>"#)
        .unwrap();
    let protocol = card_protocol(parser);
    let output = render(&protocol, &test_json!({"label":"global","labels":["loop"]})).unwrap();
    assert!(output.contains("proploopprop"), "{output}");
}

#[test]
fn structural_calls_in_attribute_templates_are_not_silently_ignored() {
    let protocol = protocol([
        ("entry", vec![F::attribute_template("title", "attribute")]),
        ("attribute", vec![F::render("attribute", "", "")]),
    ]);
    assert!(matches!(
        render(&protocol, &Value::Null),
        Err(HandlerError::Invariant(_))
    ));
}

#[test]
fn ordinary_deep_structural_graph_has_no_streaming_frame_cap() {
    let mut records = HashMap::new();
    for index in 0..600 {
        records.insert(
            index.to_string(),
            FragmentList {
                fragments: vec![F::if_cond(
                    ConditionExpr::identifier("yes"),
                    (index + 1).to_string(),
                )],
                contains_boundary: false,
            },
        );
    }

    records.insert(
        "entry".into(),
        FragmentList {
            fragments: vec![F::render("0", "", "")],
            contains_boundary: false,
        },
    );
    records.insert(
        "600".into(),
        FragmentList {
            fragments: vec![F::raw("done")],
            contains_boundary: false,
        },
    );
    assert_eq!(
        render(
            &Protocol::new(WebUIProtocol::new(records)),
            &test_json!({"yes":true})
        )
        .unwrap(),
        "done"
    );
}

#[test]
fn streaming_invocation_budget_survives_resume_and_advance() {
    let protocol = protocol([
        (
            "entry",
            vec![
                F::raw("<html><head>"),
                F::signal("}}}webui:head_start", true),
                F::raw("</head><body>"),
                F::signal("}}}webui:body_start", true),
                F::for_loop("item", "before", "call"),
                F::boundary(0, "entry", "pause", None),
                F::raw("ready"),
                F::boundary_end(0),
                F::for_loop("item", "after", "call"),
                F::signal("}}}webui:body_end", true),
                F::raw("</body></html>"),
            ],
        ),
        ("call", vec![F::render("leaf", "", "")]),
        ("leaf", vec![]),
    ]);
    let protocol = Arc::new(protocol);
    for extra in [0, 1] {
        let mut session = StreamingSession::new(
            Arc::new(WebUIHandler::new()),
            Arc::clone(&protocol),
            SessionOptions::new("entry", "/"),
        )
        .unwrap();
        let first = session
            .start(test_json!({
                "before": vec![Value::Null; MAX_FRAGMENT_INVOCATIONS / 2],
                "after": vec![Value::Null; MAX_FRAGMENT_INVOCATIONS / 2 + extra],
            }))
            .unwrap();
        session
            .resume_current(first.boundary.unwrap().instance_id, BoundaryMode::Final)
            .unwrap();
        let result = session.advance();
        if extra == 0 {
            assert!(result.unwrap().done);
        } else {
            assert!(matches!(
                result,
                Err(HandlerError::FragmentCallBudget { .. })
            ));
        }
    }
}

#[test]
fn streaming_capture_sources_are_independent_of_replaced_owner_state() {
    let handler = WebUIHandler::with_plugin(|| {
        Box::new(webui_handler::plugin::webui::WebUIHydrationPlugin::new())
    });
    let mut session = StreamingSession::new(
        Arc::new(handler), Arc::new(streaming_card(concat!(
            r#"<fragment name="body"><boundary name="ready"><p>{{row.label}}/{{owner}}</p></boundary>"#,
            r#"<footer>{{row.label}}/{{owner}}</footer></fragment>"#,
            r#"<render fragment="body" scope="{{source}}" as="row" />"#,
        ))), SessionOptions::new("entry", "/"),
    ).unwrap();
    let start = session
        .start(test_json!({
            "source":{"label":"OLD"},"owner":"before"
        }))
        .unwrap();
    assert!(String::from_utf8(start.bytes)
        .unwrap()
        .contains("<!--wf:0-->"));
    let commit = session
        .resume(
            start.boundary.unwrap().instance_id,
            test_json!({"source":{"label":"NEW"},"owner":"after"}),
            BoundaryMode::Final,
        )
        .unwrap();
    let commit = String::from_utf8(commit.bytes).unwrap();
    assert!(!commit.contains("\"fragmentInputs\""));
    // The checkpoint inside the call defines the input it selects, so the range
    // resolves its own marker at activation.
    let checkpoint = stream_records(&commit).next().unwrap();
    assert_eq!(
        checkpoint[3]["fragmentSources"],
        test_json!([[0, 0, {"label":"OLD"}]])
    );
    assert!(commit.contains("OLD"));
    assert!(commit.contains("after"));
    let tail = session.advance().unwrap();
    assert!(tail.done);
    let tail = String::from_utf8(tail.bytes).unwrap();
    let span = stream_records(&tail).find(|record| record[1] == 3).unwrap();
    assert_eq!(span[3]["fragmentSourceRefs"], test_json!([0]));
    // A definition is response-wide, so the owning span repeats only the ref.
    assert!(span[3].get("fragmentSources").is_none(), "{tail}");
    assert!(!tail.contains("\"fragmentInputs\""));
}

#[test]
fn completed_calls_keep_capture_sources_until_the_owning_span_closes() {
    let protocol = streaming_card(concat!(
        r#"<fragment name="empty"></fragment><render fragment="empty" />"#,
        r#"<fragment name="completed">{{row.label}}</fragment>"#,
        r#"<render fragment="completed" scope="{{source}}" as="row" />"#,
        r#"<boundary name="pause">{{owner}}</boundary>"#,
    ));
    let mut session = StreamingSession::new(
        Arc::new(WebUIHandler::with_plugin(|| {
            Box::new(webui_handler::plugin::webui::WebUIHydrationPlugin::new())
        })),
        Arc::new(protocol),
        SessionOptions::new("entry", "/"),
    )
    .unwrap();
    let start = session
        .start(test_json!({
            "source":{"label":"OLD"},"owner":"before"
        }))
        .unwrap();
    let bytes = String::from_utf8(start.bytes).unwrap();
    assert!(bytes.contains("<!--wf-->"), "{bytes}");
    assert!(bytes.contains("<!--wf:0-->"), "{bytes}");
    assert!(bytes.contains("OLD"), "{bytes}");
    let commit = session
        .resume(
            start.boundary.unwrap().instance_id,
            test_json!({"source":{"label":"NEW"},"owner":"after"}),
            BoundaryMode::Final,
        )
        .unwrap();
    let commit = String::from_utf8(commit.bytes).unwrap();
    assert!(!commit.contains("\"fragmentInputs\""), "{commit}");
    // The call completed before the boundary, yet its definition still reaches
    // the browser before anything can activate the host that contains it.
    let checkpoint = stream_records(&commit).next().unwrap();
    assert_eq!(
        checkpoint[3]["fragmentSources"],
        test_json!([[0, 0, {"label":"OLD"}]])
    );
    assert!(commit.contains("after"), "{commit}");
    let tail = session.advance().unwrap();
    assert!(tail.done);
    let tail = String::from_utf8(tail.bytes).unwrap();
    let span = stream_records(&tail).find(|record| record[1] == 3).unwrap();
    assert_eq!(span[3]["fragmentSourceRefs"], test_json!([0]));
}

#[test]
fn streaming_capture_refs_belong_to_the_nearest_span_and_reuse_definitions() {
    let mut parser = webui_parser::HtmlParser::new();
    for (tag, template) in [
        (
            "my-card",
            concat!(
                r#"<fragment name="empty"></fragment><render fragment="empty" />"#,
                r#"<fragment name="leaf">{{row.label}}</fragment>"#,
                r#"<render fragment="leaf" scope="{{source}}" as="row" />"#,
                "<child-card></child-card>",
                r#"<render fragment="leaf" scope="{{source}}" as="row" />"#,
                r#"<boundary name="parent">{{owner}}</boundary>"#,
            ),
        ),
        (
            "child-card",
            concat!(
                r#"<fragment name="leaf">{{row.label}}</fragment>"#,
                r#"<render fragment="leaf" scope="{{source}}" as="row" />"#,
                r#"<boundary name="child">{{owner}}</boundary>"#,
            ),
        ),
    ] {
        parser
            .component_registry_mut()
            .register_component(webui_parser::ComponentRegistration::new(
                tag, template, None, true,
            ))
            .unwrap();
    }
    parser
        .parse(
            "entry",
            "<html><head></head><body><my-card></my-card></body></html>",
        )
        .unwrap();
    let mut document = WebUIProtocol::new(parser.into_fragment_records());
    for tag in ["my-card", "child-card"] {
        document.components.insert(
            tag.into(),
            webui_protocol::ComponentData {
                uses_shadow_dom: true,
                ..Default::default()
            },
        );
    }
    document.populate_style_closures(&["entry"]);
    let mut session = StreamingSession::new(
        Arc::new(WebUIHandler::with_plugin(|| {
            Box::new(webui_handler::plugin::webui::WebUIHydrationPlugin::new())
        })),
        Arc::new(Protocol::new(document)),
        SessionOptions::new("entry", "/"),
    )
    .unwrap();
    let start = session
        .start(test_json!({
            "source":{"label":"OLD"}, "owner":"before"
        }))
        .unwrap();
    let child = start.boundary.unwrap();
    assert_eq!(child.name.as_ref(), "child");
    let mut response = String::from_utf8(start.bytes).unwrap();
    let resumed = session
        .resume_current(child.instance_id, BoundaryMode::Final)
        .unwrap();
    response.push_str(&String::from_utf8(resumed.bytes).unwrap());
    let between = session.advance().unwrap();
    let between_bytes = String::from_utf8(between.bytes).unwrap();
    response.push_str(&between_bytes);
    let child_span = stream_records(&between_bytes)
        .find(|record| record[1] == 3)
        .unwrap();
    // The nested component owns its own span, so the call inside it is retained
    // there rather than by the enclosing host.
    assert_eq!(child_span[3]["fragmentSourceRefs"], test_json!([0]));
    let parent = between.boundary.unwrap();
    assert_eq!(parent.name.as_ref(), "parent");
    let resumed = session
        .resume(
            parent.instance_id,
            test_json!({
                "source":{"label":"NEW"}, "owner":"after"
            }),
            BoundaryMode::Final,
        )
        .unwrap();
    response.push_str(&String::from_utf8(resumed.bytes).unwrap());
    let tail = session.advance().unwrap();
    assert!(tail.done);
    let bytes = String::from_utf8(tail.bytes).unwrap();
    response.push_str(&bytes);
    let parent_span = stream_records(&bytes)
        .find(|record| record[1] == 3)
        .unwrap();
    // Both parent calls captured the same value, so the host retains one ref and
    // the response never redefines it.
    assert_eq!(parent_span[3]["fragmentSourceRefs"], test_json!([0]));
    assert!(parent_span[3].get("fragmentSources").is_none(), "{bytes}");
    // The whole response defines the shared root exactly once, before the record
    // that first refers to it, and every scoped call selects it by that one id
    // while the parameterless call stays bare.
    assert_eq!(
        response
            .matches(r#""fragmentSources":[[0,0,{"label":"OLD"}]]"#)
            .count(),
        1,
        "{response}"
    );
    let defined_at = response.find(r#""fragmentSources""#).unwrap();
    let first_ref_at = response.find(r#""fragmentSourceRefs""#).unwrap();
    assert!(defined_at < first_ref_at, "{response}");
    assert_eq!(response.matches("<!--wf:0-->").count(), 3, "{response}");
    assert_eq!(response.matches("<!--wf-->").count(), 1, "{response}");
    assert!(!response.contains("<!--wf:1-->"), "{response}");
}

#[test]
fn streaming_fragment_plugin_scope_is_preserved_until_parent_tail_finishes() {
    #[derive(Default)]
    struct ScopePlugin(usize);

    impl webui_handler::plugin::HandlerPlugin for ScopePlugin {
        fn push_scope(&mut self) {
            self.0 += 1;
        }
        fn pop_scope(&mut self) {
            self.0 -= 1;
        }
        fn on_binding_start(
            &mut self,
            _: &str,
            _: bool,
            writer: &mut dyn ResponseWriter,
        ) -> webui_handler::Result<()> {
            writer.write(&self.0.to_string())?;
            writer.write(":")
        }
        fn on_binding_end(
            &mut self,
            _: &str,
            _: bool,
            _: &mut dyn ResponseWriter,
        ) -> webui_handler::Result<()> {
            Ok(())
        }
        fn on_repeat_item_start(
            &mut self,
            _: usize,
            _: &mut dyn ResponseWriter,
        ) -> webui_handler::Result<()> {
            Ok(())
        }
        fn on_repeat_item_end(
            &mut self,
            _: usize,
            _: &mut dyn ResponseWriter,
        ) -> webui_handler::Result<()> {
            Ok(())
        }
        fn on_element_data(
            &mut self,
            _: &[u8],
            _: &mut dyn ResponseWriter,
        ) -> webui_handler::Result<()> {
            Ok(())
        }
    }

    let mut session = StreamingSession::new(
        Arc::new(WebUIHandler::with_plugin(|| Box::<ScopePlugin>::default())),
        Arc::new(streaming_protocol()),
        SessionOptions::new("entry", "/"),
    )
    .unwrap();
    let start = session
        .start(test_json!({
            "source":{"label":"OLD"},"owner":"before"
        }))
        .unwrap();
    let commit = session
        .resume(
            start.boundary.unwrap().instance_id,
            test_json!({"source":{"label":"NEW"},"owner":"after"}),
            BoundaryMode::Final,
        )
        .unwrap();
    assert!(String::from_utf8(commit.bytes)
        .unwrap()
        .contains("<p>1:OLD/1:after</p>"));
    let tail = String::from_utf8(session.advance().unwrap().bytes).unwrap();
    assert!(tail.contains("<footer>1:OLD/1:after</footer><aside>0:NEW</aside>"));
}

#[test]
fn streaming_capture_lineage_uses_item_tuples_instead_of_indexed_paths() {
    let protocol = streaming_card(concat!(
        r#"<fragment name="cell">{{cell.label}}</fragment>"#,
        r#"<for each="row in rows"><render fragment="cell" scope="{{row.child}}" as="cell" /></for>"#,
        r#"<boundary name="pause">{{owner}}</boundary>"#,
    ));
    let mut session = StreamingSession::new(
        Arc::new(WebUIHandler::with_plugin(|| {
            Box::new(webui_handler::plugin::webui::WebUIHydrationPlugin::new())
        })),
        Arc::new(protocol),
        SessionOptions::new("entry", "/"),
    )
    .unwrap();
    let start = session
        .start(test_json!({
            "rows":[{"child":{"label":"A"}},{"child":{"label":"B"}}],
            "owner":"before"
        }))
        .unwrap();
    let bytes = String::from_utf8(start.bytes).unwrap();
    // Each loop item selects its own id; the shared array is captured once.
    assert!(bytes.contains("<!--wf:2-->"), "{bytes}");
    assert!(bytes.contains("<!--wf:4-->"), "{bytes}");
    let commit = session
        .resume(
            start.boundary.unwrap().instance_id,
            test_json!({
                "rows":[{"child":{"label":"A"}},{"child":{"label":"B"}}],
                "owner":"after"
            }),
            BoundaryMode::Final,
        )
        .unwrap();
    let commit = String::from_utf8(commit.bytes).unwrap();
    let checkpoint = stream_records(&commit).next().unwrap();
    // An array step is its own `[id,2,parent,index]` tuple, so a projection path
    // never carries a numeric segment the browser decoder would reject.
    assert_eq!(
        checkpoint[3]["fragmentSources"],
        test_json!([
            [0, 0, [{"child":{"label":"A"}},{"child":{"label":"B"}}]],
            [1, 2, 0, 0],
            [2, 1, 1, "child"],
            [3, 2, 0, 1],
            [4, 1, 3, "child"]
        ]),
        "{commit}"
    );
    assert!(!commit.contains("rows.0"), "{commit}");
}

#[test]
fn first_component_prop_is_captured_for_internal_scoped_calls() {
    let mut parser = webui_parser::HtmlParser::new();
    parser
        .component_registry_mut()
        .register_component(webui_parser::ComponentRegistration::new(
            "my-card",
            concat!(
                r#"<fragment name="row">{{row.label}}/{{extra.label}}</fragment>"#,
                r#"<render fragment="row" scope="{{model}}" as="row" />"#,
                r#"<boundary name="pause">{{owner}}</boundary>"#,
            ),
            None,
            true,
        ))
        .unwrap();
    parser
        .parse(
            "entry",
            concat!(
                "<html><head></head><body>",
                r#"<my-card :model="{{source}}" :extra="{{source}}"></my-card>"#,
                "</body></html>",
            ),
        )
        .unwrap();
    let protocol = card_protocol(parser);
    let mut session = StreamingSession::new(
        Arc::new(WebUIHandler::with_plugin(|| {
            Box::new(webui_handler::plugin::webui::WebUIHydrationPlugin::new())
        })),
        Arc::new(protocol),
        SessionOptions::new("entry", "/"),
    )
    .unwrap();
    let start = session
        .start(test_json!({"source":{"label":"OLD"},"owner":"before"}))
        .unwrap();
    let bytes = String::from_utf8(start.bytes).unwrap();
    // The first prop of the element carries `attr_start`; it must still be a
    // durable input, not a borrowed value the call cannot retain.
    assert!(bytes.contains("<!--wf:0-->"), "{bytes}");
    assert!(bytes.contains("OLD/OLD"), "{bytes}");
    let commit = session
        .resume(
            start.boundary.unwrap().instance_id,
            test_json!({"source":{"label":"NEW"},"owner":"after"}),
            BoundaryMode::Final,
        )
        .unwrap();
    let commit = String::from_utf8(commit.bytes).unwrap();
    let checkpoint = stream_records(&commit).next().unwrap();
    assert_eq!(
        checkpoint[3]["fragmentSources"],
        test_json!([[0, 0, {"label":"OLD"}]]),
        "{commit}"
    );
}

#[test]
fn loop_member_capture_falls_back_to_owner_state_like_ordinary_rendering() {
    let template = concat!(
        r#"<fragment name="cell">{{cell}}|{{item.fallback}}</fragment>"#,
        r#"<for each="item in items"><render fragment="cell" scope="{{item.fallback}}" as="cell" /></for>"#,
        r#"<boundary name="pause">{{owner}}</boundary>"#,
    );
    let state = test_json!({
        "items":[{}], "item":{"fallback":"GLOBAL"}, "owner":"before"
    });
    let ordinary = render(&streaming_card(template), &state).unwrap();
    assert!(ordinary.contains("GLOBAL|GLOBAL"), "{ordinary}");

    let mut session = StreamingSession::new(
        Arc::new(WebUIHandler::with_plugin(|| {
            Box::new(webui_handler::plugin::webui::WebUIHydrationPlugin::new())
        })),
        Arc::new(streaming_card(template)),
        SessionOptions::new("entry", "/"),
    )
    .unwrap();
    let start = session.start(state).unwrap();
    let bytes = String::from_utf8(start.bytes).unwrap();
    // A loop root is not an alias: a missing member still resolves against the
    // owner state, and the captured input is the owner value it resolved to.
    assert!(bytes.contains("GLOBAL|GLOBAL"), "{bytes}");
    assert!(bytes.contains("<!--wf:"), "{bytes}");
}

#[test]
fn repeated_captures_retain_each_source_once_in_first_use_order() {
    let protocol = streaming_card(concat!(
        r#"<fragment name="cell">{{cell.label}}</fragment>"#,
        r#"<render fragment="cell" scope="{{a}}" as="cell" />"#,
        r#"<render fragment="cell" scope="{{b}}" as="cell" />"#,
        r#"<render fragment="cell" scope="{{a}}" as="cell" />"#,
        r#"<render fragment="cell" scope="{{b}}" as="cell" />"#,
        r#"<boundary name="pause">{{owner}}</boundary>"#,
    ));
    let mut session = StreamingSession::new(
        Arc::new(WebUIHandler::with_plugin(|| {
            Box::new(webui_handler::plugin::webui::WebUIHydrationPlugin::new())
        })),
        Arc::new(protocol),
        SessionOptions::new("entry", "/"),
    )
    .unwrap();
    let start = session
        .start(test_json!({
            "a":{"label":"A"},"b":{"label":"B"},"owner":"before"
        }))
        .unwrap();
    let bytes = String::from_utf8(start.bytes).unwrap();
    assert_eq!(bytes.matches("<!--wf:0-->").count(), 2, "{bytes}");
    assert_eq!(bytes.matches("<!--wf:1-->").count(), 2, "{bytes}");
    let commit = session
        .resume(
            start.boundary.unwrap().instance_id,
            test_json!({"a":{"label":"A2"},"b":{"label":"B2"},"owner":"after"}),
            BoundaryMode::Final,
        )
        .unwrap();
    let tail = session.advance().unwrap();
    assert!(tail.done);
    let mut response = String::from_utf8(commit.bytes).unwrap();
    response.push_str(&String::from_utf8(tail.bytes).unwrap());
    let span = stream_records(&response)
        .find(|record| record[1] == 3)
        .unwrap();
    // Four calls, two distinct inputs: each identifier is retained once, in the
    // order the span first needed it.
    assert_eq!(
        span[3]["fragmentSourceRefs"],
        test_json!([0, 1]),
        "{response}"
    );
}

#[test]
fn synchronous_streaming_keeps_borrowed_inputs_and_emits_no_source_table() {
    let protocol = streaming_card(concat!(
        r#"<fragment name="row">{{row.label}}</fragment>"#,
        r#"<render fragment="row" scope="{{source}}" as="row" />"#,
        r#"<boundary name="pause">{{owner}}</boundary>"#,
    ));
    let mut writer = Writer::default();
    WebUIHandler::with_plugin(|| {
        Box::new(webui_handler::plugin::webui::WebUIHydrationPlugin::new())
    })
    .render_streaming(
        &protocol,
        &test_json!({"source":{"label":"OLD"},"owner":"before"}),
        &RenderOptions::new("entry", "/"),
        &mut writer,
    )
    .unwrap();
    let output = writer.0;
    // One state renders the whole response, so no call can outlive the value it
    // rendered: the client resolves the alias from the same caller state. A
    // marker identifier without a definition would be hard client skew.
    assert!(output.contains("<!--wf-->"), "{output}");
    assert!(!output.contains("<!--wf:"), "{output}");
    assert!(!output.contains("fragmentSources"), "{output}");
    assert!(!output.contains("fragmentSourceRefs"), "{output}");
    assert!(output.contains("OLD"), "{output}");
}

#[test]
fn streaming_owned_fragment_inputs_only_emit_host_driven_provenance() {
    for (call, expected, value) in [
        (
            r#"<render fragment="row" scope="{{items.length}}" as="row" />"#,
            "3",
            test_json!(3),
        ),
        (
            r#"<render fragment="row" scope="{{text.length}}" as="row" />"#,
            "6",
            test_json!(6),
        ),
        (
            r#"<render fragment="row" scope="{{label}}" as="row" />"#,
            "local",
            test_json!("local"),
        ),
    ] {
        let mut template = concat!(
            r#"<fragment name="row"><span class="before">{{row}}</span>"#,
            r#"<boundary name="pause"><span class="during">{{row}}</span></boundary>"#,
            r#"<span class="after">{{row}}</span></fragment>"#,
        )
        .to_owned();
        template.push_str(call);
        let mut parser = webui_parser::HtmlParser::with_plugin(Box::new(
            webui_parser::plugin::webui::WebUIParserPlugin::new(),
        ));
        parser
            .component_registry_mut()
            .register_component(webui_parser::ComponentRegistration::new(
                "my-card", &template, None, true,
            ))
            .unwrap();
        parser
            .parse(
                "entry",
                concat!(
                    "<html><head></head><body>",
                    r#"<my-card label="local"></my-card>"#,
                    "</body></html>",
                ),
            )
            .unwrap();
        let document = card_document(parser);
        let metadata: Value =
            serde_json::from_str(&document.components["my-card"].template_json).unwrap();
        assert_eq!(metadata["u"].as_array().unwrap().len(), 1);
        let protocol = Arc::new(Protocol::new(document));

        for host_driven in [false, true] {
            let state = test_json!({
                "items": [null, false, "item"],
                "text": "\u{e9}\u{1f600}",
                "label": "owner"
            });
            let handler = WebUIHandler::with_plugin(|| {
                Box::new(webui_handler::plugin::webui::WebUIHydrationPlugin::new())
            });
            let output = if host_driven {
                let mut session = StreamingSession::new(
                    Arc::new(handler),
                    Arc::clone(&protocol),
                    SessionOptions::new("entry", "/"),
                )
                .unwrap();
                let start = session.start(state).unwrap();
                let mut output = String::from_utf8(start.bytes).unwrap();
                let commit = session
                    .resume(
                        start.boundary.unwrap().instance_id,
                        test_json!({"items": [], "text": "", "label": "replacement"}),
                        BoundaryMode::Final,
                    )
                    .unwrap();
                output.push_str(&String::from_utf8(commit.bytes).unwrap());
                let tail = session.advance().unwrap();
                assert!(tail.done);
                output.push_str(&String::from_utf8(tail.bytes).unwrap());
                output
            } else {
                let mut writer = Writer::default();
                handler
                    .render_streaming(
                        &protocol,
                        &state,
                        &RenderOptions::new("entry", "/"),
                        &mut writer,
                    )
                    .unwrap();
                writer.0
            };
            for phase in ["before", "during", "after"] {
                let rendered = format!(r#"<span class="{phase}">{expected}</span>"#);
                assert!(
                    output.contains(&rendered),
                    "{call}, {host_driven}: {output}"
                );
            }
            let checkpoint = stream_records(&output)
                .find(|record| record[1] == 0)
                .unwrap();
            let span = stream_records(&output)
                .find(|record| record[1] == 3)
                .unwrap();
            assert!(
                stream_records(&output).any(|record| record[3]["templates"]["my-card"].is_object()),
                "{output}"
            );
            if host_driven {
                assert_eq!(output.matches("<!--wf:0-->").count(), 1, "{output}");
                assert!(!output.contains("<!--wf-->"), "{output}");
                assert_eq!(
                    checkpoint[3]["fragmentSources"],
                    test_json!([[0, 0, &value]]),
                    "{output}"
                );
                assert_eq!(span[3]["fragmentSourceRefs"], test_json!([0]), "{output}");
            } else {
                assert_eq!(output.matches("<!--wf-->").count(), 1, "{output}");
                assert!(!output.contains("<!--wf:"), "{output}");
                assert!(!output.contains("fragmentSources"), "{output}");
                assert!(!output.contains("fragmentSourceRefs"), "{output}");
            }
        }
    }
}

#[test]
fn span_completion_primes_its_host_props_ahead_of_caller_state() {
    let mut parser = webui_parser::HtmlParser::new();
    parser
        .component_registry_mut()
        .register_component(webui_parser::ComponentRegistration::new(
            "my-card",
            concat!(
                r#"<fragment name="row">"#,
                r#"<boundary name="prop-ready">{{row.name}}/{{title}}</boundary>"#,
                r#"</fragment>"#,
                r#"<render fragment="row" scope="{{model}}" as="row" />"#,
                r#"<output>{{selected}}</output>"#,
            ),
            None,
            true,
        ))
        .unwrap();
    parser
        .parse(
            "entry",
            concat!(
                "<html><head></head><body>",
                r#"<my-card :model="{{source}}" :title="{{title}}"></my-card>"#,
                "</body></html>",
            ),
        )
        .unwrap();
    let protocol = card_protocol(parser);
    let mut session = StreamingSession::new(
        Arc::new(WebUIHandler::with_plugin(|| {
            Box::new(webui_handler::plugin::webui::WebUIHydrationPlugin::new())
        })),
        Arc::new(protocol),
        SessionOptions::new("entry", "/"),
    )
    .unwrap();
    let start = session
        .start(test_json!({
            "source":{"name":"Captured OLD"},
            "title":"Stream forest",
            "selected":""
        }))
        .unwrap();
    let mut response = String::from_utf8(start.bytes).unwrap();
    let commit = session
        .resume(
            start.boundary.unwrap().instance_id,
            test_json!({
                "source":{"name":"Prop replacement"},
                "title":"Prop owner",
                "selected":""
            }),
            BoundaryMode::Final,
        )
        .unwrap();
    response.push_str(&String::from_utf8(commit.bytes).unwrap());
    let tail = session.advance().unwrap();
    assert!(tail.done);
    response.push_str(&String::from_utf8(tail.bytes).unwrap());

    // The suspended body renders with the props the host was opened with.
    assert!(
        response.contains("Captured OLD/Stream forest"),
        "{response}"
    );
    let span = stream_records(&response)
        .find(|record| record[1] == 3)
        .expect("span completion record");
    // The host rendered with the props it was opened with, so its own record
    // primes those and never the caller state the resume installed.
    assert_eq!(
        span[3]["state"]["model"],
        test_json!({"name":"Captured OLD"}),
        "{response}"
    );
    assert_eq!(
        span[3]["state"]["title"],
        test_json!("Stream forest"),
        "{response}"
    );
    assert_eq!(span[3]["state"]["selected"], test_json!(""), "{response}");
    assert!(span[3]["stateRef"].is_null(), "{response}");

    // The caller's own record keeps the state the host resumed with.
    let checkpoint = stream_records(&response)
        .find(|record| record[1] == 0 || record[1] == 1)
        .expect("boundary checkpoint record");
    assert_eq!(
        checkpoint[3]["state"]["title"],
        test_json!("Prop owner"),
        "{response}"
    );
    assert!(checkpoint[3]["state"]["model"].is_null(), "{response}");
}

#[test]
fn a_record_after_a_primed_span_resends_state_instead_of_referencing_it() {
    let mut parser = webui_parser::HtmlParser::new();
    parser
        .component_registry_mut()
        .register_component(webui_parser::ComponentRegistration::new(
            "my-card",
            concat!(
                r#"<fragment name="row">"#,
                r#"<boundary name="prop-ready" key="{{row.name}}">{{row.name}}/{{title}}</boundary>"#,
                r#"</fragment>"#,
                r#"<render fragment="row" scope="{{model}}" as="row" />"#,
            ),
            None,
            true,
        ))
        .unwrap();
    parser
        .parse(
            "entry",
            concat!(
                "<html><head></head><body>",
                r#"<my-card :model="{{source}}" :title="{{title}}"></my-card>"#,
                r#"<my-card :model="{{other}}" :title="{{title}}"></my-card>"#,
                "</body></html>",
            ),
        )
        .unwrap();
    let protocol = card_protocol(parser);
    let mut session = StreamingSession::new(
        Arc::new(WebUIHandler::with_plugin(|| {
            Box::new(webui_handler::plugin::webui::WebUIHydrationPlugin::new())
        })),
        Arc::new(protocol),
        SessionOptions::new("entry", "/"),
    )
    .unwrap();
    let start = session
        .start(test_json!({
            "source":{"name":"FIRST"},
            "other":{"name":"SECOND"},
            "title":"Stream forest"
        }))
        .unwrap();
    let mut response = String::from_utf8(start.bytes).unwrap();
    let mut pending = start.boundary.map(|boundary| boundary.instance_id);
    while let Some(instance_id) = pending.take() {
        let step = session
            .resume(
                instance_id,
                test_json!({
                    "source":{"name":"NEW FIRST"},
                    "other":{"name":"NEW SECOND"},
                    "title":"Prop owner"
                }),
                BoundaryMode::Final,
            )
            .unwrap();
        response.push_str(&String::from_utf8(step.bytes).unwrap());
        let step = session.advance().unwrap();
        response.push_str(&String::from_utf8(step.bytes).unwrap());
        pending = step.boundary.map(|boundary| boundary.instance_id);
        if step.done {
            break;
        }
    }
    let records: Vec<Value> = stream_records(&response).collect();
    let mut primed = Vec::new();
    let mut previous: Option<u64> = None;
    for record in &records {
        let sequence = record[0].as_u64().unwrap();
        if let Some(reference) = record[3]["stateRef"].as_u64() {
            // The browser keeps exactly one base — the previous range record —
            // so a reference to anything else is a hard client error.
            assert_eq!(Some(reference), previous, "{response}");
            assert!(!primed.contains(&reference), "{response}");
        }
        if record[3]["state"]["model"].is_null() {
            assert!(record[1] != 3 || record[3]["state"].is_null(), "{response}");
        } else {
            primed.push(sequence);
        }
        if record[1] != 4 {
            previous = Some(sequence);
        }
    }
    assert_eq!(primed.len(), 2, "{response}");
    let spans: Vec<&Value> = records.iter().filter(|record| record[1] == 3).collect();
    assert_eq!(spans.len(), 2, "{response}");
    // Each host is primed with the props it was actually opened with: the
    // second element only started rendering after the first resume.
    assert_eq!(
        spans[0][3]["state"]["model"],
        test_json!({"name":"FIRST"}),
        "{response}"
    );
    assert_eq!(
        spans[0][3]["state"]["title"],
        test_json!("Stream forest"),
        "{response}"
    );
    assert_eq!(
        spans[1][3]["state"]["model"],
        test_json!({"name":"NEW SECOND"}),
        "{response}"
    );
    assert_eq!(
        spans[1][3]["state"]["title"],
        test_json!("Prop owner"),
        "{response}"
    );
    // The record that follows a primed span cannot inherit it, so it carries a
    // complete projection of its own.
    let after = records
        .iter()
        .position(|record| record[1] == 3)
        .and_then(|index| records.get(index + 1))
        .expect("a record after the first span");
    assert!(after[3]["stateRef"].is_null(), "{response}");
    assert!(after[3]["state"].is_object(), "{response}");
}
