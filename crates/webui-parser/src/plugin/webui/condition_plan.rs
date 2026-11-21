// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::emit_js_string;
use std::fmt::Write as _;
use webui_protocol::{condition_expr::Expr, ConditionExpr, LogicalOperator, PredicateOperand};

const PATH_OPERAND: i32 = 8;
const CONSTANT: i32 = 7;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    False,
    True,
    Test(usize),
}

impl std::fmt::Display for Target {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::False => out.write_str("-1"),
            Self::True => out.write_str("-2"),
            Self::Test(index) => index.fmt(out),
        }
    }
}

enum Operand<'a> {
    Path(usize),
    Literal(&'a str),
    Integer(i64),
    Number(f64),
    Bool(bool),
}

struct Test<'a> {
    op: i32,
    left: usize,
    right: Operand<'a>,
    yes: Target,
    no: Target,
}

enum Pending<'a> {
    Test(Option<&'a ConditionExpr>, Target, Target),
    Left(Option<&'a ConditionExpr>, bool, Target, Target),
}

pub(super) fn emit(condition: &ConditionExpr, out: &mut String) {
    let (expr, yes, _) = strip_negation(Some(condition), Target::True, Target::False);
    if let Some(Expr::Identifier(id)) = expr {
        if id.value != "true" && id.value != "false" {
            out.push_str(if yes == Target::True {
                "[[0],["
            } else {
                "[[1],["
            });
            emit_js_string(&id.value, out);
            out.push_str("]]");
            return;
        }
    }
    let mut paths = Vec::with_capacity(2);
    let mut tests = Vec::with_capacity(1);
    let mut pending = vec![Pending::Test(Some(condition), Target::True, Target::False)];
    let mut entry = Target::False;
    while let Some(item) = pending.pop() {
        let (expr, yes, no) = match item {
            Pending::Test(expr, yes, no) => (expr, yes, no),
            Pending::Left(expr, and, yes, no) => {
                if and {
                    (expr, entry, no)
                } else {
                    (expr, yes, entry)
                }
            }
        };
        let (expr, yes, no) = strip_negation(expr, yes, no);
        match expr {
            Some(Expr::Compound(compound)) => {
                pending.push(Pending::Left(
                    compound.left.as_deref(),
                    compound.op == LogicalOperator::And as i32,
                    yes,
                    no,
                ));
                pending.push(Pending::Test(compound.right.as_deref(), yes, no));
            }
            leaf => {
                let (op, left, right) = compile_leaf(leaf, &mut paths);
                // Every target is already emitted or a terminal, so the browser
                // can walk this plan without a stack or a cycle.
                entry = Target::Test(tests.len());
                tests.push(Test {
                    op,
                    left,
                    right,
                    yes,
                    no,
                });
            }
        }
    }

    out.push_str("[[");
    for (index, test) in tests.iter().enumerate() {
        if index != 0 {
            out.push(',');
        }
        let _ = write!(out, "{},{},", test.op, test.left);
        match test.right {
            Operand::Path(slot) => {
                let _ = write!(out, "{slot}");
            }
            Operand::Literal(text) => emit_js_string(text, out),
            Operand::Integer(number) => {
                let _ = write!(out, "{number}");
            }
            Operand::Number(number) => {
                let _ = write!(out, "{number}");
            }
            Operand::Bool(value) => out.push_str(if value { "true" } else { "false" }),
        }
        if tests.len() != 1 || test.yes != Target::True {
            let _ = write!(out, ",{},{}", test.yes, test.no);
        }
    }
    out.push_str("],[");
    for (index, path) in paths.iter().enumerate() {
        if index != 0 {
            out.push(',');
        }
        emit_js_string(path, out);
    }
    out.push_str("]]");
}

fn strip_negation(
    mut condition: Option<&ConditionExpr>,
    mut yes: Target,
    mut no: Target,
) -> (Option<&Expr>, Target, Target) {
    loop {
        match condition.and_then(|expr| expr.expr.as_ref()) {
            Some(Expr::Not(not)) => {
                std::mem::swap(&mut yes, &mut no);
                condition = not.condition.as_deref();
            }
            expr => return (expr, yes, no),
        }
    }
}

fn compile_leaf<'a>(expr: Option<&'a Expr>, paths: &mut Vec<&'a str>) -> (i32, usize, Operand<'a>) {
    match expr {
        Some(Expr::Identifier(id)) if id.value == "true" || id.value == "false" => {
            (CONSTANT, 0, Operand::Bool(id.value == "true"))
        }
        Some(Expr::Identifier(id)) => (0, slot(paths, &id.value), Operand::Bool(false)),
        Some(Expr::Predicate(predicate)) => {
            let left = slot(paths, &predicate.left);
            let (flag, right) = match PredicateOperand::classify(&predicate.right) {
                PredicateOperand::Path(path) => (PATH_OPERAND, Operand::Path(slot(paths, path))),
                PredicateOperand::String(text) => (0, Operand::Literal(text)),
                PredicateOperand::Bool(value) => (0, Operand::Bool(value)),
                PredicateOperand::Integer(value) => (0, Operand::Integer(value)),
                PredicateOperand::Float(value) if value.is_finite() => (0, Operand::Number(value)),
                _ => return (CONSTANT, 0, Operand::Bool(false)),
            };
            (predicate.operator | flag, left, right)
        }
        _ => (CONSTANT, 0, Operand::Bool(false)),
    }
}

fn slot<'a>(paths: &mut Vec<&'a str>, path: &'a str) -> usize {
    if let Some(index) = paths.iter().position(|existing| *existing == path) {
        return index;
    }
    let index = paths.len();
    paths.push(path);
    index
}

#[cfg(test)]
mod tests {
    use super::*;
    use webui_protocol::ComparisonOperator;

    #[test]
    fn predicates_distinguish_numeric_and_quoted_literals_from_paths() {
        for (right, expected) in [
            ("010", r#"[[3,0,10],["count"]]"#),
            ("1e2", r#"[[3,0,100],["count"]]"#),
            ("'010'", r#"[[3,0,"010"],["count"]]"#),
            ("+1", r#"[[11,0,1],["count","+1"]]"#),
            (".5", r#"[[11,0,1],["count",".5"]]"#),
            ("limits.count", r#"[[11,0,1],["count","limits.count"]]"#),
        ] {
            let mut out = String::new();
            emit(
                &ConditionExpr::predicate("count", ComparisonOperator::Equal, right),
                &mut out,
            );
            assert_eq!(out, expected);
        }
    }

    #[test]
    fn negation_swaps_terminal_targets_in_single_and_nested_tests() {
        use ConditionExpr as C;
        for (condition, expected) in [
            (
                C::negated(C::predicate("count", ComparisonOperator::Equal, "100")),
                r#"[[3,0,100,-1,-2],["count"]]"#,
            ),
            (
                C::negated(C::identifier("false")),
                r#"[[7,0,false,-1,-2],[]]"#,
            ),
            (
                C::negated(C::compound(
                    C::identifier("visible"),
                    LogicalOperator::Or,
                    C::compound(
                        C::identifier("enabled"),
                        LogicalOperator::Or,
                        C::negated(C::identifier("ready")),
                    ),
                )),
                r#"[[0,0,false,-2,-1,0,1,false,-1,0,0,2,false,-1,1],["ready","enabled","visible"]]"#,
            ),
        ] {
            let mut out = String::new();
            emit(&condition, &mut out);
            assert_eq!(out, expected);
        }
    }
}
