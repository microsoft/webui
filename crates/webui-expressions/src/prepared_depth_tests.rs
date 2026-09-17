// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;

#[test]
fn five_operators_and_negation_at_every_tree_level() {
    for op in [LogicalOperator::And, LogicalOperator::Or] {
        for flags in 0u8..64 {
            for negations in 0u8..64 {
                let state = test_json!({
                    "a": flags & 1 != 0, "b": flags & 2 != 0,
                    "c": flags & 4 != 0, "d": flags & 8 != 0,
                    "e": flags & 16 != 0, "f": flags & 32 != 0,
                });
                let mut condition = ConditionExpr::identifier("a");
                let mut expected = flags & 1 != 0;
                for (index, path) in ["b", "c", "d", "e", "f"].iter().enumerate() {
                    if negations & (1 << index) != 0 {
                        condition = ConditionExpr::negated(condition);
                        expected = !expected;
                    }
                    let value = flags & (1 << (index + 1)) != 0;
                    expected = if op == LogicalOperator::And {
                        expected && value
                    } else {
                        expected || value
                    };
                    let leaf = ConditionExpr::identifier(*path);
                    condition = if flags & (1 << index) != 0 {
                        ConditionExpr::compound(condition, op, leaf)
                    } else {
                        ConditionExpr::compound(leaf, op, condition)
                    };
                }
                if negations & 32 != 0 {
                    condition = ConditionExpr::negated(condition);
                    expected = !expected;
                }
                assert_eq!(assert_parity(&condition, &state).0, Ok(expected));
            }
        }
    }
}

#[test]
fn negated_sibling_subtrees_keep_their_own_short_circuit_boundaries() {
    for op in [LogicalOperator::And, LogicalOperator::Or] {
        for bits in 0u8..16 {
            let state = test_json!({
                "a": bits & 1 != 0, "b": bits & 2 != 0,
                "c": bits & 4 != 0, "d": bits & 8 != 0,
            });
            let condition = ConditionExpr::compound(
                ConditionExpr::negated(ConditionExpr::compound(
                    ConditionExpr::identifier("a"),
                    op,
                    ConditionExpr::identifier("b"),
                )),
                op,
                ConditionExpr::negated(ConditionExpr::compound(
                    ConditionExpr::identifier("c"),
                    op,
                    ConditionExpr::identifier("d"),
                )),
            );
            let expected = if op == LogicalOperator::And {
                !(bits & 3 == 3) && !(bits & 12 == 12)
            } else {
                !(bits & 3 != 0) || !(bits & 12 != 0)
            };
            assert_eq!(assert_parity(&condition, &state).0, Ok(expected));
        }
    }
}

fn deep_not(mut condition: ConditionExpr, depth: usize) -> ConditionExpr {
    for _ in 0..depth {
        condition = ConditionExpr::negated(condition);
    }
    condition
}

// Generated protobuf drop is recursive; keep that separate from the evaluator
// under test by dismantling intentionally extreme wire fixtures iteratively.
fn drop_wire_tree(condition: ConditionExpr) {
    let mut pending = vec![condition];
    while let Some(condition) = pending.pop() {
        match condition.expr {
            Some(Expr::Not(mut not)) => {
                if let Some(inner) = not.condition.take() {
                    pending.push(*inner);
                }
            }
            Some(Expr::Compound(mut compound)) => {
                if let Some(left) = compound.left.take() {
                    pending.push(*left);
                }
                if let Some(right) = compound.right.take() {
                    pending.push(*right);
                }
            }
            _ => {}
        }
    }
}

#[test]
fn deep_not_compilation_evaluation_and_prepared_drop_do_not_recurse() {
    let condition = deep_not(
        ConditionExpr::compound(
            deep_not(ConditionExpr::identifier("false"), 20_001),
            LogicalOperator::And,
            deep_not(ConditionExpr::identifier("true"), 20_000),
        ),
        100_001,
    );
    assert_eq!(assert_parity(&condition, &Value::Null), (Ok(false), vec![]));
    drop_wire_tree(condition);
    let condition = deep_not(absent_not(), 100_000);
    assert!(assert_parity(&condition, &Value::Null).0.is_err());
    drop_wire_tree(condition);
}
