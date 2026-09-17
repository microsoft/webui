// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::mem::{size_of, size_of_val};

use super::{Operand, PreparedCondition, PreparedKind, Step, Test};
use webui_protocol::{ComparisonOperator, ConditionExpr, LogicalOperator};

fn retained_payload(prepared: &PreparedCondition) -> usize {
    let tape = match &prepared.kind {
        PreparedKind::Constant(_) => return size_of::<PreparedCondition>(),
        PreparedKind::Identifier { path, .. } => {
            return size_of::<PreparedCondition>() + path.len()
        }
        PreparedKind::Tape(tape) => tape,
    };
    let literal_bytes: usize = tape
        .steps
        .iter()
        .map(|step| match &step.test {
            Test::Predicate {
                right: Operand::Literal(Ok(serde_json::Value::String(value))),
                ..
            } => value.capacity(),
            _ => 0,
        })
        .sum();
    size_of::<PreparedCondition>()
        + size_of_val(tape.steps.as_ref())
        + tape.paths.len()
        + literal_bytes
}

#[test]
fn report_retained_payload_and_bounded_tape() {
    let fixtures = [
        ("constant", ConditionExpr::identifier("true")),
        ("identifier", ConditionExpr::identifier("isAdmin")),
        (
            "string_predicate",
            ConditionExpr::predicate("status", ComparisonOperator::Equal, "'active'"),
        ),
        (
            "and_2_terms",
            ConditionExpr::compound(
                ConditionExpr::identifier("isAdmin"),
                LogicalOperator::And,
                ConditionExpr::identifier("isActive"),
            ),
        ),
        (
            "admin_check",
            ConditionExpr::compound(
                ConditionExpr::predicate("user.role", ComparisonOperator::Equal, "'admin'"),
                LogicalOperator::And,
                ConditionExpr::negated(ConditionExpr::identifier("user.suspended")),
            ),
        ),
    ];
    println!(
        "PreparedCondition={} bytes; Step={} bytes",
        size_of::<PreparedCondition>(),
        size_of::<Step>()
    );
    for (name, condition) in fixtures {
        let prepared = PreparedCondition::new(&condition);
        println!(
            "{name}: retained payload={} bytes",
            retained_payload(&prepared)
        );
        if let PreparedKind::Tape(tape) = &prepared.kind {
            assert!(tape.steps.len() <= 2);
        }
    }
    let condition = ConditionExpr::identifier("flag");
    let plain = PreparedCondition::new(&condition);
    let negated = PreparedCondition::new(&ConditionExpr::negated(condition));
    assert_eq!(retained_payload(&plain), retained_payload(&negated));

    // An accepted five-compound wire tree has at most six terminal tests;
    // NOT chains and valid logical operators never add retained instructions.
    let mut condition = ConditionExpr::identifier("flag");
    for _ in 0..5 {
        condition = ConditionExpr::compound(
            condition,
            LogicalOperator::And,
            ConditionExpr::identifier("flag"),
        );
    }
    assert!(matches!(
        PreparedCondition::new(&condition).kind,
        PreparedKind::Tape(tape) if tape.steps.len() == 6
    ));
    // Rejected global validation retains just its error, not the input paths.
    condition = ConditionExpr::compound(
        condition,
        LogicalOperator::And,
        ConditionExpr::identifier("flag"),
    );
    let prepared = PreparedCondition::new(&condition);
    assert!(matches!(
        prepared.kind,
        PreparedKind::Tape(tape) if tape.steps.len() == 1 && tape.paths.is_empty()
    ));
}
