// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::borrow::Cow;

use serde_json::Value;
use webui_protocol::{condition_expr::Expr, ConditionExpr, LogicalOperator};

use crate::{
    evaluate_predicate, evaluation_error, logical_operator, truthy, ExpressionError, Result,
};

pub(crate) const MAX_OPERATORS: usize = 5;

// Strip arbitrary NOT depth without using a traversal or call stack.
pub(crate) fn strip_nots(mut condition: &ConditionExpr) -> (&ConditionExpr, bool) {
    let mut negated = false;
    while let Some(Expr::Not(not)) = &condition.expr {
        let Some(inner) = not.condition.as_deref() else {
            break;
        };
        condition = inner;
        negated = !negated;
    }
    (condition, negated)
}

// The guard is global, including malformed/skipped children and NOT subtrees.
pub(crate) fn validate(condition: &ConditionExpr) -> Result<()> {
    let mut count = 0;
    let mut last_op = None;
    let mut mixed = false;
    let mut stack = Vec::with_capacity(MAX_OPERATORS + 1);
    stack.push(condition);
    while let Some(condition) = stack.pop() {
        let (condition, _) = strip_nots(condition);
        if let Some(Expr::Compound(compound)) = &condition.expr {
            count += 1;
            if let Some(last) = last_op {
                mixed |= last != compound.op;
            } else {
                last_op = Some(compound.op);
            }
            if let Some(right) = compound.right.as_deref() {
                stack.push(right);
            }
            if let Some(left) = compound.left.as_deref() {
                stack.push(left);
            }
        }
    }
    if count > MAX_OPERATORS {
        return Err(ExpressionError::TooManyOperators(count));
    }
    if mixed {
        return Err(ExpressionError::MixedOperators);
    }
    Ok(())
}

#[inline]
pub(crate) fn evaluate_term<'a, F>(condition: &ConditionExpr, resolver: &F) -> Result<bool>
where
    F: Fn(&str) -> Option<Cow<'a, Value>>,
{
    match &condition.expr {
        Some(Expr::Predicate(predicate)) => evaluate_predicate(predicate, resolver),
        Some(Expr::Identifier(id)) => match id.value.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            path => Ok(resolver(path).is_some_and(|value| truthy(value.as_ref()))),
        },
        Some(Expr::Not(_)) => Err(evaluation_error("Not condition missing inner expression")),
        None => Err(evaluation_error("Empty condition expression")),
        Some(Expr::Compound(_)) => Err(evaluation_error("Expected a terminal expression")),
    }
}

// Keep traversal setup out of the inlinable single-term entry point.
#[inline(never)]
pub(crate) fn evaluate_nested<'a, F>(condition: &ConditionExpr, resolver: &F) -> Result<bool>
where
    F: Fn(&str) -> Option<Cow<'a, Value>>,
{
    let (condition, negated) = strip_nots(condition);
    let result = if matches!(condition.expr, Some(Expr::Compound(_))) {
        validate(condition)?;
        evaluate_tree(condition, resolver)?
    } else {
        evaluate_term(condition, resolver)?
    };
    Ok(result ^ negated)
}

#[derive(Clone, Copy)]
struct Frame<'a> {
    right: &'a ConditionExpr,
    operator: i32,
    negated: bool,
}

// Called only after the global guard has bounded compound depth to five.
pub(crate) fn evaluate_tree<'a, F>(mut condition: &ConditionExpr, resolver: &F) -> Result<bool>
where
    F: Fn(&str) -> Option<Cow<'a, Value>>,
{
    let mut stack = [None; MAX_OPERATORS];
    let mut depth = 0;
    let mut negate_result = false;
    'descend: loop {
        let (inner, negated) = strip_nots(condition);
        condition = inner;
        negate_result ^= negated;
        if let Some(Expr::Compound(compound)) = &condition.expr {
            // Both children must exist even when the right one would be skipped.
            let left = compound
                .left
                .as_deref()
                .ok_or_else(|| evaluation_error("Compound missing left expression"))?;
            let right = compound
                .right
                .as_deref()
                .ok_or_else(|| evaluation_error("Compound missing right expression"))?;
            let slot = stack.get_mut(depth).ok_or_else(|| {
                evaluation_error("Expression traversal exceeded validated operator limit")
            })?;
            *slot = Some(Frame {
                right,
                operator: compound.op,
                negated: negate_result,
            });
            depth += 1;
            condition = left;
            negate_result = false;
            continue;
        }

        let mut result = evaluate_term(condition, resolver)? ^ negate_result;
        while depth != 0 {
            depth -= 1;
            let frame = stack[depth]
                .take()
                .ok_or_else(|| evaluation_error("Expression traversal missing continuation"))?;
            // Invalid operators are errors only after the left side succeeds.
            let operator = logical_operator(frame.operator)?;
            let short_circuit = match operator {
                LogicalOperator::And => !result,
                LogicalOperator::Or => result,
                LogicalOperator::Unspecified => {
                    return Err(evaluation_error("Unspecified logical operator"));
                }
            };
            if short_circuit {
                result ^= frame.negated;
            } else {
                condition = frame.right;
                negate_result = frame.negated;
                continue 'descend;
            }
        }
        return Ok(result);
    }
}
