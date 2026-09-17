// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

//! Both condition modes through the serialized server pipeline and continuation VM.

use std::sync::Arc;

use serde_json::Value;
use webui_handler::{
    BoundaryMode, ConditionEvaluation, HandlerError, Protocol, ProtocolOptions, RenderOptions,
    ResponseWriter, SessionOptions, StreamingSession, WebUIHandler,
};
use webui_parser::{ComponentRegistration, DomStrategy, HtmlParser};
use webui_protocol::{
    condition_expr::Expr, web_ui_fragment::Fragment, ComparisonOperator, CompoundCondition,
    ConditionExpr, IdentifierCondition, LogicalOperator, NotCondition, Predicate, WebUIProtocol,
};
use webui_test_utils::test_json;

fn parsed_data(source: &str, components: &[(&str, &str)]) -> WebUIProtocol {
    let mut parser = HtmlParser::with_options(DomStrategy::Light);
    for (tag, template) in components {
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(tag, template, None, true))
            .unwrap();
    }
    parser.parse("index.html", source).unwrap();
    let mut data = WebUIProtocol::new(parser.into_fragment_records());
    data.populate_style_closures(&["index.html"]);
    data
}

fn round_trip(data: &WebUIProtocol, condition_evaluation: ConditionEvaluation) -> Arc<Protocol> {
    Arc::new(
        Protocol::from_protobuf_with_options(
            &data.to_protobuf().unwrap(),
            ProtocolOptions {
                condition_evaluation,
            },
        )
        .unwrap(),
    )
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

fn render(protocol: &Protocol, state: &Value) -> Result<String, HandlerError> {
    let mut writer = StringWriter::default();
    WebUIHandler::new().render(
        protocol,
        state,
        &RenderOptions::new("index.html", "/"),
        &mut writer,
    )?;
    Ok(writer.output)
}

macro_rules! condition_mode_tests {
    ($($scenario:ident),+ $(,)?) => {
        mod prepared {
            $(
                #[test]
                fn $scenario() {
                    super::$scenario(super::ConditionEvaluation::Prepared);
                }
            )+
        }

        mod direct {
            $(
                #[test]
                fn $scenario() {
                    super::$scenario(super::ConditionEvaluation::Direct);
                }
            )+
        }
    };
}

condition_mode_tests!(
    condition_variants_and_boolean_attributes_follow_each_render_state,
    identifier_truthiness_preserves_empty_and_nonempty_value_boundaries,
    missing_comparison_operands_differ_from_missing_bare_identifiers,
    loop_dotted_paths_and_component_props_do_not_leak_between_items_or_renders,
    negation_does_not_turn_missing_comparison_errors_into_true,
    malformed_rhs_is_lazy_but_a_missing_rhs_node_is_an_error,
    progressive_conditions_resume_with_fresh_state_and_updates_are_patch_records,
);

fn condition_variants_and_boolean_attributes_follow_each_render_state(mode: ConditionEvaluation) {
    let initial = test_json!({
        "ready": true, "busy": false, "fallback": false,
        "stock": 3, "minimum": 1, "name": "Zoë", "status": "ready", "discount": -1
    });
    let changed = test_json!({
        "ready": false, "busy": true, "fallback": false,
        "stock": 0, "minimum": 1, "name": "", "status": "pending", "discount": -3
    });
    for condition in [
        "ready",
        "!busy",
        "ready && !busy",
        "ready || fallback",
        "stock > 0",
        "stock >= 3",
        "stock == 3",
        "stock != 0",
        "minimum < stock",
        "minimum <= stock",
        "name.length > 2",
        "status == 'ready'",
        "ready == true",
        "discount > -2.5",
    ] {
        let source = format!(
            r#"<if condition="{condition}"><p>selected</p></if><button ?disabled="{{{{{condition}}}}}">Save</button>"#
        );
        let protocol = round_trip(&parsed_data(&source, &[]), mode);
        // Reuse the loaded protocol, including a return to the original state.
        for (state, expected) in [
            (&initial, "<p>selected</p><button disabled>Save</button>"),
            (&changed, "<button>Save</button>"),
            (&initial, "<p>selected</p><button disabled>Save</button>"),
        ] {
            assert_eq!(render(&protocol, state).unwrap(), expected, "{condition}");
        }
    }
}

fn identifier_truthiness_preserves_empty_and_nonempty_value_boundaries(mode: ConditionEvaluation) {
    let protocol = round_trip(
        &parsed_data(
            r#"<if condition="value">yes</if><if condition="!value">no</if><input ?required="{{value}}">"#,
            &[],
        ),
        mode,
    );
    for (value, truthy) in [
        (test_json!(null), false),
        (test_json!(false), false),
        (test_json!(0), false),
        (test_json!(""), false),
        (test_json!([]), false),
        (test_json!({}), false),
        (test_json!(true), true),
        (test_json!(-1), true),
        (test_json!("false"), true),
        (test_json!([0]), true),
        (test_json!({"enabled": false}), true),
    ] {
        assert_eq!(
            render(&protocol, &test_json!({"value": value})).unwrap(),
            if truthy {
                "yes<input required>"
            } else {
                "no<input>"
            },
            "value: {value}"
        );
    }
    assert_eq!(render(&protocol, &test_json!({})).unwrap(), "no<input>");
}

fn missing_comparison_operands_differ_from_missing_bare_identifiers(mode: ConditionEvaluation) {
    let state = test_json!({"ready": true, "count": 3});
    for (condition, selected) in [
        ("missing", false),
        ("!missing", true),
        ("missing > 0", false),
        ("count > missing", false),
        ("missing > 0 || ready", false),
        ("ready || missing > 0", true),
        ("missing && ready", false),
        ("missing || ready", true),
        ("!missing && ready", true),
    ] {
        let source = format!(
            r#"<if condition="{condition}">yes</if><input ?disabled="{{{{{condition}}}}}">"#
        );
        let protocol = round_trip(&parsed_data(&source, &[]), mode);
        assert_eq!(
            render(&protocol, &state).unwrap(),
            if selected {
                "yes<input disabled>"
            } else {
                "<input>"
            },
            "{condition}"
        );
    }
}

fn loop_dotted_paths_and_component_props_do_not_leak_between_items_or_renders(
    mode: ConditionEvaluation,
) {
    let protocol = round_trip(
        &parsed_data(
            concat!(
                r#"<for each="item in items">"#,
                r#"<if condition="item.profile.active && item.quantity >= minimum"><b>{{item.profile.name}}</b></if>"#,
                r#"<stock-card :product="{{item}}" :minimum="{{minimum}}"></stock-card>"#,
                "</for>",
                r#"<if condition="item.profile.active"><footer>global restored</footer></if>"#,
            ),
            &[(
                "stock-card",
                concat!(
                    r#"<if condition="product.profile.active && product.quantity >= minimum"><span>{{product.profile.name}}</span></if>"#,
                    r#"<button ?disabled="{{!product.profile.active}}">{{product.profile.name}}</button>"#,
                ),
            )],
        ),
        mode,
    );
    let state = test_json!({
        "minimum": 2,
        "item": {"profile": {"active": true}},
        "product": {"profile": {"active": false, "name": "wrong global"}, "quantity": 99},
        "items": [
            {"profile": {"active": true, "name": "Café & tea"}, "quantity": 2},
            {"profile": {"active": false, "name": "Sold out"}, "quantity": 0},
            {"profile": {"name": "Draft"}, "quantity": 5}
        ]
    });
    let expected = concat!(
        "<b>Café &amp; tea</b><stock-card><span>Café &amp; tea</span><button>Café &amp; tea</button></stock-card>",
        "<stock-card><button disabled>Sold out</button></stock-card>",
        // A missing loop-local dotted path falls back to the global item path.
        // Component props resolve independently against product, not item.
        "<b>Draft</b><stock-card><button disabled>Draft</button></stock-card>",
        "<footer>global restored</footer>",
    );
    assert_eq!(render(&protocol, &state).unwrap(), expected);
    assert_eq!(
        render(
            &protocol,
            &test_json!({"items": [], "item": {"profile": {"active": false}}})
        )
        .unwrap(),
        ""
    );
    assert_eq!(render(&protocol, &state).unwrap(), expected);
}

fn identifier(value: &str) -> ConditionExpr {
    ConditionExpr {
        expr: Some(Expr::Identifier(IdentifierCondition {
            value: value.into(),
        })),
    }
}

fn predicate(left: &str, operator: i32, right: &str) -> ConditionExpr {
    ConditionExpr {
        expr: Some(Expr::Predicate(Predicate {
            left: left.into(),
            operator,
            right: right.into(),
        })),
    }
}

fn compound(op: LogicalOperator, right: Option<ConditionExpr>) -> ConditionExpr {
    ConditionExpr {
        expr: Some(Expr::Compound(Box::new(CompoundCondition {
            left: Some(Box::new(identifier("ready"))),
            op: op as i32,
            right: right.map(Box::new),
        }))),
    }
}

fn raw_condition_protocol(condition: ConditionExpr, mode: ConditionEvaluation) -> Arc<Protocol> {
    // Keep the parsed fragment graph and real wire boundary; replace only the
    // AST to exercise inputs that valid template syntax cannot express.
    let mut data = parsed_data(
        "<i>before</i><if condition=\"ready\"><b>selected</b></if><u>after</u>",
        &[],
    );
    let entry = data.fragments.get_mut("index.html").unwrap();
    let target = entry
        .fragments
        .iter_mut()
        .find_map(|fragment| match &mut fragment.fragment {
            Some(Fragment::IfCond(target)) => Some(target),
            _ => None,
        })
        .unwrap();
    target.condition = Some(condition);
    round_trip(&data, mode)
}

fn negation_does_not_turn_missing_comparison_errors_into_true(mode: ConditionEvaluation) {
    for (left, right) in [("missing", "0"), ("count", "missing")] {
        let protocol = raw_condition_protocol(
            ConditionExpr {
                expr: Some(Expr::Not(Box::new(NotCondition {
                    condition: Some(Box::new(predicate(
                        left,
                        ComparisonOperator::GreaterThan as i32,
                        right,
                    ))),
                }))),
            },
            mode,
        );
        assert_eq!(
            render(&protocol, &test_json!({"count": 3})).unwrap(),
            "<i>before</i><u>after</u>"
        );
    }
}

fn malformed_rhs_is_lazy_but_a_missing_rhs_node_is_an_error(mode: ConditionEvaluation) {
    for (rhs, message) in [
        (ConditionExpr { expr: None }, "Empty condition expression"),
        (
            predicate("count", 99, "0"),
            "Invalid comparison operator: 99",
        ),
        (
            predicate("count", ComparisonOperator::Equal as i32, "1oops"),
            "Invalid literal: 1oops",
        ),
        (
            ConditionExpr {
                expr: Some(Expr::Not(Box::new(NotCondition { condition: None }))),
            },
            "Not condition missing inner expression",
        ),
    ] {
        for (op, skip_state, expected) in [
            (
                LogicalOperator::Or,
                true,
                "<i>before</i><b>selected</b><u>after</u>",
            ),
            (LogicalOperator::And, false, "<i>before</i><u>after</u>"),
        ] {
            let protocol = raw_condition_protocol(compound(op, Some(rhs.clone())), mode);
            let skipped = test_json!({"ready": skip_state, "count": 3});
            assert_eq!(render(&protocol, &skipped).unwrap(), expected);
            let error =
                render(&protocol, &test_json!({"ready": !skip_state, "count": 3})).unwrap_err();
            assert!(matches!(error, HandlerError::Evaluation(_)), "{error}");
            assert!(error.to_string().contains(message), "{error}");
            assert_eq!(render(&protocol, &skipped).unwrap(), expected);
        }
    }
    for (op, ready) in [(LogicalOperator::Or, true), (LogicalOperator::And, false)] {
        let protocol = raw_condition_protocol(compound(op, None), mode);
        assert!(render(&protocol, &test_json!({"ready": ready}))
            .unwrap_err()
            .to_string()
            .contains("Compound missing right expression"));
    }
}

fn progressive_conditions_resume_with_fresh_state_and_updates_are_patch_records(
    mode: ConditionEvaluation,
) {
    let protocol = round_trip(
        &parsed_data(
            concat!(
                "<html><head></head><body>",
                r#"<if condition="ready && count >= 0 && !busy"><boundary name="live">"#,
                r#"<button ?disabled="{{!available}}">Buy</button><if condition="available && count > 0"><p>{{count}} available</p></if>"#,
                "</boundary></if>",
                r#"<if condition="later || count > 0"><boundary name="next">"#,
                r#"<button ?disabled="{{!available}}">Next</button><if condition="available"><p>available</p></if>"#,
                "</boundary></if><footer>tail</footer></body></html>",
            ),
            &[],
        ),
        mode,
    );
    let new_session = || {
        StreamingSession::new(
            Arc::new(WebUIHandler::new()),
            Arc::clone(&protocol),
            SessionOptions::new("index.html", "/"),
        )
        .unwrap()
    };
    let mut session = new_session();
    // The selected if body contains a boundary: discovery must use VM begin_if,
    // not just the boundary-free ordinary-render fast path.
    let start = session
        .start(test_json!({
            "ready": true, "busy": false, "available": false, "count": 0, "later": false
        }))
        .unwrap();
    assert!(!start.done);
    let live = start.boundary.unwrap();
    assert_eq!(live.name.as_ref(), "live");
    assert_eq!(
        String::from_utf8(start.bytes).unwrap(),
        r#"<html><head><meta name="webui-streaming" content="1"></head><body>"#
    );
    let committed = session
        .resume(
            live.instance_id,
            test_json!({
                "available": true, "count": 2, "later": true
            }),
            BoundaryMode::Updatable,
        )
        .unwrap();
    assert!(!committed.done && committed.boundary.is_none());
    let committed = String::from_utf8(committed.bytes).unwrap();
    assert!(
        committed.starts_with("<!--wb:0--><button>Buy</button><p>2 available</p><!--/wb:0-->"),
        "{committed}"
    );
    assert!(!committed.contains("<footer>"));
    // An invalid patch is rejected before writing and does not poison recovery.
    assert!(session.update(live.instance_id, &test_json!([])).is_err());
    let update = String::from_utf8(
        session
            .update(
                live.instance_id,
                &test_json!({
                    "available": false, "count": 0, "later": false
                }),
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        update,
        r#"<script type="application/json" data-webui-boundary>[1,2,0,{"available":false,"count":0,"later":false}]</script><webui-hydrate></webui-hydrate>"#
    );
    // Updates target the committed browser range; they do not replace the
    // retained server snapshot. The later condition sees the resume state.
    let next = session.advance().unwrap();
    assert!(!next.done);
    let next = next.boundary.unwrap();
    assert_eq!(next.name.as_ref(), "next");
    let committed = session
        .resume(
            next.instance_id,
            test_json!({
                "available": false, "count": 0
            }),
            BoundaryMode::Final,
        )
        .unwrap();
    assert!(!committed.done && committed.boundary.is_none());
    let committed = String::from_utf8(committed.bytes).unwrap();
    assert!(
        committed.starts_with("<!--wb:1--><button disabled>Next</button><!--/wb:1-->"),
        "{committed}"
    );
    let end = session.advance().unwrap();
    assert!(end.done && end.boundary.is_none());
    assert!(String::from_utf8(end.bytes)
        .unwrap()
        .contains("<footer>tail</footer>"));
    let hidden = new_session()
        .start(test_json!({"ready": false, "later": false}))
        .unwrap();
    assert!(hidden.done && hidden.boundary.is_none());
    let hidden = String::from_utf8(hidden.bytes).unwrap();
    assert!(!hidden.contains("<button"));
    assert!(!hidden.contains("<!--wb:"));
    assert!(hidden.contains("<footer>tail</footer>"));
}
