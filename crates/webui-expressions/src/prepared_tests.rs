// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use std::{borrow::Cow, cell::RefCell};

use serde_json::Value;
use webui_protocol::{
    condition_expr::Expr, ComparisonOperator, CompoundCondition, ConditionExpr, LogicalOperator,
    NotCondition, Predicate,
};
use webui_state::find_value_by_dotted_path_ref;
use webui_test_utils::test_json;

use crate::{evaluate_with_resolver, PreparedCondition};

#[path = "prepared_depth_tests.rs"]
mod depth;

type Observation = (std::result::Result<bool, String>, Vec<String>);

fn observe(condition: &ConditionExpr, state: &Value, prepared: bool) -> Observation {
    let calls = RefCell::new(Vec::new());
    let resolver = |path: &str| {
        calls.borrow_mut().push(path.to_owned());
        find_value_by_dotted_path_ref(path, state)
    };
    let result = if prepared {
        PreparedCondition::new(condition).evaluate_with_resolver(resolver)
    } else {
        evaluate_with_resolver(condition, resolver)
    };
    (
        result.map_err(|error| format!("{error:?}")),
        calls.into_inner(),
    )
}

fn assert_parity(condition: &ConditionExpr, state: &Value) -> Observation {
    let expected = observe(condition, state, false);
    assert_eq!(observe(condition, state, true), expected);
    assert_eq!(
        PreparedCondition::new(condition)
            .evaluate(state)
            .map_err(|e| format!("{e:?}")),
        expected.0
    );
    expected
}

fn predicate(left: &str, operator: i32, right: &str) -> ConditionExpr {
    ConditionExpr {
        expr: Some(Expr::Predicate(Predicate {
            left: left.to_owned(),
            operator,
            right: right.to_owned(),
        })),
    }
}

fn compound(left: Option<ConditionExpr>, op: i32, right: Option<ConditionExpr>) -> ConditionExpr {
    ConditionExpr {
        expr: Some(Expr::Compound(Box::new(CompoundCondition {
            left: left.map(Box::new),
            op,
            right: right.map(Box::new),
        }))),
    }
}

fn absent_not() -> ConditionExpr {
    ConditionExpr {
        expr: Some(Expr::Not(Box::new(NotCondition { condition: None }))),
    }
}

#[test]
fn truthiness_constants_and_borrowed_or_synthetic_resolver_values() {
    for value in [
        Value::Null,
        test_json!(false),
        test_json!(true),
        test_json!(0),
        test_json!(-0.0),
        test_json!(1),
        test_json!(-1),
        test_json!(""),
        test_json!("false"),
        test_json!([]),
        test_json!([false]),
        test_json!({}),
        test_json!({"key": null}),
    ] {
        let state = test_json!({"value": value, "true": false, "false": true});
        for path in ["value", "missing", "true", "false"] {
            let mut condition = ConditionExpr::identifier(path);
            for _ in 0..4 {
                let _ = assert_parity(&condition, &state);
                condition = ConditionExpr::negated(condition);
            }
        }
    }
    for literal in ["true", "false"] {
        let condition = ConditionExpr::identifier(literal);
        assert_eq!(
            assert_parity(&condition, &Value::Null).1,
            Vec::<String>::new()
        );
    }
    let prepared = PreparedCondition::new(&ConditionExpr::identifier("value"));
    let value = Value::Bool(true);
    assert!(prepared
        .evaluate_with_resolver(|_| Some(Cow::Borrowed(&value)))
        .unwrap());
    assert!(!prepared
        .evaluate_with_resolver(|_| Some(Cow::Owned(Value::Bool(false))))
        .unwrap());
    assert!(!prepared.evaluate_with_resolver(|_| None).unwrap());
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<PreparedCondition>();
}

#[test]
fn predicates_match_for_all_operators_literals_and_value_types() {
    for value in [
        Value::Null,
        test_json!(false),
        test_json!(true),
        test_json!(0),
        test_json!(25),
        test_json!(1.25),
        test_json!("25"),
        test_json!("text"),
        test_json!([]),
        test_json!({"x": true}),
    ] {
        let state = test_json!({"left": value, "right": value, "items": [1, 2]});
        for left in ["left", "missing", "items.length"] {
            for right in [
                "right",
                "missing",
                "true",
                "false",
                "null",
                "25",
                "-1",
                "1.25",
                "1e3",
                "1e999",
                "2bad",
                "'text'",
                "\"text\"",
                "''",
                "\"\"",
                "'",
                "\"",
                "'unclosed",
                "'é'",
                "items.length",
                "-",
            ] {
                for op in [-1, 0, 1, 2, 3, 4, 5, 6, 7, i32::MAX] {
                    let _ = assert_parity(&predicate(left, op, right), &state);
                }
            }
        }
    }
}

#[test]
fn malformed_terms_and_nested_short_circuits_match_direct_evaluation() {
    let leaves = [
        ConditionExpr::identifier("yes"),
        ConditionExpr::identifier("no"),
        ConditionExpr::identifier("missing"),
        ConditionExpr::identifier("true"),
        ConditionExpr::default(),
        absent_not(),
        predicate("value", 3, "'ok'"),
        predicate("missing", 3, "'"),
        predicate("value", -1, "other"),
        predicate("value", 3, "2bad"),
        compound(None, 1, Some(ConditionExpr::identifier("yes"))),
        compound(Some(ConditionExpr::identifier("no")), 1, None),
    ];
    let state = test_json!({"yes": true, "no": false, "value": "ok", "other": 1});
    for left in &leaves {
        for right in &leaves {
            for op in [-1, 0, 1, 2, 17] {
                for left_not in [false, true] {
                    let left = if left_not {
                        ConditionExpr::negated(left.clone())
                    } else {
                        left.clone()
                    };
                    let mut condition = compound(Some(left), op, Some(right.clone()));
                    for _ in 0..3 {
                        let _ = assert_parity(&condition, &state);
                        condition = ConditionExpr::negated(condition);
                    }
                }
            }
        }
    }
}

#[test]
fn resolver_trace_and_error_precedence_are_preserved() {
    let state = test_json!({"yes": true, "no": false, "value": 1, "other": 2});
    let cases = [
        (
            predicate("missing", -1, "'"),
            "MissingValue(\"missing\")",
            vec!["missing"],
        ),
        (
            predicate("value", -1, "2bad"),
            "TypeError(\"Invalid literal: 2bad\")",
            vec!["value"],
        ),
        (
            predicate("value", -1, "missing"),
            "MissingValue(\"missing\")",
            vec!["value", "missing"],
        ),
        (
            predicate("value", -1, "other"),
            "Evaluation(\"Invalid comparison operator: -1\")",
            vec!["value", "other"],
        ),
        (
            compound(
                Some(predicate("missing", 3, "0")),
                -1,
                Some(ConditionExpr::identifier("yes")),
            ),
            "MissingValue(\"missing\")",
            vec!["missing"],
        ),
        (
            compound(
                Some(ConditionExpr::identifier("no")),
                -1,
                Some(ConditionExpr::identifier("yes")),
            ),
            "Evaluation(\"Invalid logical operator: -1\")",
            vec!["no"],
        ),
        (
            compound(
                Some(ConditionExpr::identifier("no")),
                0,
                Some(ConditionExpr::identifier("yes")),
            ),
            "Evaluation(\"Unspecified logical operator\")",
            vec!["no"],
        ),
        (
            compound(Some(ConditionExpr::identifier("no")), 1, None),
            "Evaluation(\"Compound missing right expression\")",
            vec![],
        ),
        (
            compound(None, -1, None),
            "Evaluation(\"Compound missing left expression\")",
            vec![],
        ),
    ];
    for (condition, error, calls) in cases {
        let observed = assert_parity(&condition, &state);
        assert_eq!(observed.0, Err(error.to_owned()));
        assert_eq!(observed.1, calls);
    }
    for (op, left, expected) in [(1, "no", false), (2, "yes", true)] {
        for malformed in [
            predicate("missing", -1, "'"),
            absent_not(),
            ConditionExpr::default(),
        ] {
            let condition = compound(Some(ConditionExpr::identifier(left)), op, Some(malformed));
            assert_eq!(
                assert_parity(&condition, &state),
                (Ok(expected), vec![left.to_owned()])
            );
        }
    }
}

#[test]
fn one_character_quotes_are_recoverable_and_lazy() {
    for right in ["'", "\""] {
        let condition = predicate("value", 3, right);
        assert!(matches!(
            PreparedCondition::new(&condition).evaluate(&test_json!({"value": ""})),
            Err(crate::ExpressionError::TypeError(_))
        ));
        let _ = assert_parity(&condition, &test_json!({"value": ""}));
        let condition = ConditionExpr::compound(
            ConditionExpr::identifier("false"),
            LogicalOperator::And,
            condition,
        );
        assert_eq!(assert_parity(&condition, &Value::Null).0, Ok(false));
    }
}

#[test]
fn global_validation_includes_skipped_and_malformed_subtrees() {
    for count in [1, 4, 5, 6, 20] {
        let mut right = predicate("missing", -1, "'");
        for _ in 0..count {
            right = ConditionExpr::negated(ConditionExpr::compound(
                ConditionExpr::identifier("true"),
                LogicalOperator::Or,
                right,
            ));
        }
        let condition = compound(Some(ConditionExpr::identifier("false")), 1, Some(right));
        let (result, calls) = assert_parity(&condition, &Value::Null);
        assert!(calls.is_empty());
        let expected = if count + 1 > 5 {
            format!("TooManyOperators({})", count + 1)
        } else {
            "MixedOperators".to_owned()
        };
        assert_eq!(result, Err(expected));
    }
    let condition = compound(None, -1, Some(compound(None, 1, None)));
    assert_eq!(
        assert_parity(&condition, &Value::Null),
        (Err("MixedOperators".to_owned()), vec![])
    );
}

#[test]
fn prepared_owns_paths_and_literals_independently_of_wire_and_state() {
    let prepared = {
        let condition = ConditionExpr::predicate("status", ComparisonOperator::Equal, "'active'");
        PreparedCondition::new(&condition)
    };
    assert!(prepared
        .evaluate(&test_json!({"status": "active"}))
        .unwrap());
    assert!(!prepared
        .evaluate(&test_json!({"status": "inactive"}))
        .unwrap());
    assert!(prepared.evaluate(&Value::Null).is_err());
    assert!(prepared
        .evaluate(&test_json!({"status": "active"}))
        .unwrap());
}
