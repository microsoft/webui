// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! WebUI expression evaluation module.
//!
//! This module handles the evaluation of condition expressions in WebUI templates.

use std::borrow::Cow;

use serde_json::Value;
use thiserror::Error;
use webui_protocol::{
    condition_expr, ComparisonOperator, ConditionExpr, LogicalOperator, Predicate,
};
use webui_state::find_value_by_dotted_path_ref;

const INLINE_EXPRESSION_STACK: usize = 16;

struct InlineStack<T: Copy, const N: usize> {
    inline: [Option<T>; N],
    inline_len: usize,
    overflow: Vec<T>,
}

impl<T: Copy, const N: usize> InlineStack<T, N> {
    #[inline]
    fn new(value: T) -> Self {
        let mut inline = [None; N];
        inline[0] = Some(value);
        Self {
            inline,
            inline_len: 1,
            overflow: Vec::new(),
        }
    }

    #[inline]
    fn push(&mut self, value: T) {
        if self.inline_len < N && self.overflow.is_empty() {
            self.inline[self.inline_len] = Some(value);
            self.inline_len += 1;
        } else {
            self.overflow.push(value);
        }
    }

    #[inline]
    fn pop(&mut self) -> Option<T> {
        if let Some(value) = self.overflow.pop() {
            return Some(value);
        }
        self.inline_len = self.inline_len.checked_sub(1)?;
        self.inline[self.inline_len].take()
    }
}

/// Error types for expression evaluation.
#[derive(Debug, Error)]
pub enum ExpressionError {
    #[error("Evaluation error: {0}")]
    Evaluation(String),

    #[error("Missing value: {0}")]
    MissingValue(String),

    #[error("Type error: {0}")]
    TypeError(String),
}

pub type Result<T> = std::result::Result<T, ExpressionError>;

/// Evaluate a condition expression with the given state.
///
/// Missing identifier paths are falsy operands. Missing values used by
/// comparison predicates remain evaluation errors.
///
/// Assumes the expression already passed
/// [`ConditionExpr::validate_structure`], which `webui build` guarantees for
/// parsed templates.
pub fn evaluate(condition: &ConditionExpr, state: &Value) -> Result<bool> {
    evaluate_with_resolver(condition, |path| find_value_by_dotted_path_ref(path, state))
}

/// Evaluate a condition expression using a custom resolver for value lookups.
///
/// The `resolver` closure takes a dotted path (e.g., `"contact.name"`) and
/// returns the resolved value. This allows callers to provide merged views
/// (e.g., local variables overlaid on global state) without cloning the
/// entire state tree.
pub fn evaluate_with_resolver<'a, F>(condition: &ConditionExpr, resolver: F) -> Result<bool>
where
    F: Fn(&str) -> Option<Cow<'a, Value>>,
{
    // Leaf conditions dominate real templates; dispatch them directly instead
    // of paying for the continuation stack.
    match &condition.expr {
        Some(condition_expr::Expr::Identifier(identifier)) => {
            return Ok(resolve_truthiness(&identifier.value, &resolver));
        }
        Some(condition_expr::Expr::Predicate(predicate)) => {
            return evaluate_predicate(predicate, &resolver);
        }
        _ => {}
    }

    evaluate_tree(condition, &resolver)
}

#[inline(never)]
fn evaluate_tree<'a, F>(condition: &ConditionExpr, resolver: &F) -> Result<bool>
where
    F: Fn(&str) -> Option<Cow<'a, Value>>,
{
    // Structural rules (operator count, no mixed `&&`/`||`) are invariant for a
    // given tree, so they are enforced once by `webui build` via
    // `ConditionExpr::validate_structure` rather than re-derived per render.
    evaluate_expr(condition, resolver)
}

#[derive(Clone, Copy)]
enum EvalTask<'a> {
    Evaluate(&'a ConditionExpr),
    Negate,
    ApplyCompound { op: i32, right: &'a ConditionExpr },
}

// Iterative evaluation of expressions using a resolver closure.
#[inline(never)]
fn evaluate_expr<'a, F>(condition: &ConditionExpr, resolver: &F) -> Result<bool>
where
    F: Fn(&str) -> Option<Cow<'a, Value>>,
{
    let mut tasks = InlineStack::<_, INLINE_EXPRESSION_STACK>::new(EvalTask::Evaluate(condition));
    let mut result = None;

    while let Some(task) = tasks.pop() {
        match task {
            EvalTask::Evaluate(condition) => match &condition.expr {
                Some(condition_expr::Expr::Predicate(predicate)) => {
                    result = Some(evaluate_predicate(predicate, resolver)?);
                }
                Some(condition_expr::Expr::Not(not_condition)) => {
                    let inner = not_condition.condition.as_ref().ok_or_else(|| {
                        ExpressionError::Evaluation(
                            "Not condition missing inner expression".to_string(),
                        )
                    })?;
                    tasks.push(EvalTask::Negate);
                    tasks.push(EvalTask::Evaluate(inner));
                }
                Some(condition_expr::Expr::Compound(compound)) => {
                    let left = compound.left.as_ref().ok_or_else(|| {
                        ExpressionError::Evaluation("Compound missing left expression".to_string())
                    })?;
                    let right = compound.right.as_ref().ok_or_else(|| {
                        ExpressionError::Evaluation("Compound missing right expression".to_string())
                    })?;
                    tasks.push(EvalTask::ApplyCompound {
                        op: compound.op,
                        right,
                    });
                    tasks.push(EvalTask::Evaluate(left));
                }
                Some(condition_expr::Expr::Identifier(identifier)) => {
                    result = Some(resolve_truthiness(&identifier.value, resolver));
                }
                None => {
                    return Err(ExpressionError::Evaluation(
                        "Empty condition expression".to_string(),
                    ))
                }
            },
            EvalTask::Negate => {
                result = Some(!result.ok_or_else(missing_evaluation_result_error)?);
            }
            EvalTask::ApplyCompound { op, right } => {
                let left = result.ok_or_else(missing_evaluation_result_error)?;
                let op = LogicalOperator::try_from(op).map_err(|_| {
                    ExpressionError::Evaluation(format!("Invalid logical operator: {op}"))
                })?;
                match op {
                    LogicalOperator::And if left => tasks.push(EvalTask::Evaluate(right)),
                    LogicalOperator::Or if !left => tasks.push(EvalTask::Evaluate(right)),
                    LogicalOperator::And | LogicalOperator::Or => result = Some(left),
                    LogicalOperator::Unspecified => {
                        return Err(ExpressionError::Evaluation(
                            "Unspecified logical operator".to_string(),
                        ))
                    }
                }
            }
        }
    }

    result.ok_or_else(missing_evaluation_result_error)
}

#[inline]
fn resolve_truthiness<'a, F>(path: &str, resolver: &F) -> bool
where
    F: Fn(&str) -> Option<Cow<'a, Value>>,
{
    resolver(path).is_some_and(|value| match value.as_ref() {
        Value::Bool(value) => *value,
        Value::Null => false,
        Value::Number(value) => !(value.as_f64() == Some(0.0)),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    })
}

#[cold]
#[inline(never)]
fn missing_evaluation_result_error() -> ExpressionError {
    ExpressionError::Evaluation("Expression evaluation produced no result".to_string())
}

fn evaluate_predicate<'a, F>(predicate: &Predicate, resolver: &F) -> Result<bool>
where
    F: Fn(&str) -> Option<Cow<'a, Value>>,
{
    let left_val = match resolver(&predicate.left) {
        Some(val) => val,
        None => return Err(ExpressionError::MissingValue(predicate.left.clone())),
    };

    let right = if is_literal(&predicate.right) {
        PredicateRight::Literal(parse_literal(&predicate.right)?)
    } else {
        match resolver(&predicate.right) {
            Some(value) => PredicateRight::Resolved(value),
            None => return Err(ExpressionError::MissingValue(predicate.right.clone())),
        }
    };

    let op = ComparisonOperator::try_from(predicate.operator).map_err(|_| {
        ExpressionError::Evaluation(format!(
            "Invalid comparison operator: {}",
            predicate.operator
        ))
    })?;

    compare_values(left_val.as_ref(), &op, &right)
}

// Check if a string is a literal value
fn is_literal(s: &str) -> bool {
    // A string is a literal if:
    // - It starts with a quote (single or double)
    // - It's a number (starts with a digit or negative sign followed by a digit)
    // - It's a boolean ("true" or "false")
    s.starts_with('"')
        || s.starts_with('\'')
        || s.starts_with(|c: char| {
            c.is_ascii_digit()
                || (c == '-' && s.len() > 1 && s.chars().nth(1).is_some_and(|c| c.is_ascii_digit()))
        })
        || s == "true"
        || s == "false"
}

enum PredicateLiteral<'a> {
    String(&'a str),
    Number(serde_json::Number),
    Bool(bool),
}

enum PredicateRight<'a> {
    Literal(PredicateLiteral<'a>),
    Resolved(Cow<'a, Value>),
}

// Parse a literal without allocating string values on the request path.
fn parse_literal(s: &str) -> Result<PredicateLiteral<'_>> {
    // Handle quoted strings
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        let content = &s[1..s.len() - 1];
        return Ok(PredicateLiteral::String(content));
    }

    // Handle booleans
    if s == "true" {
        return Ok(PredicateLiteral::Bool(true));
    }
    if s == "false" {
        return Ok(PredicateLiteral::Bool(false));
    }

    // Handle numbers
    if let Ok(num) = s.parse::<i64>() {
        return Ok(PredicateLiteral::Number(num.into()));
    }

    if let Ok(num) = s.parse::<f64>() {
        // Create a JSON number from f64, handling error if it's not representable
        match serde_json::Number::from_f64(num) {
            Some(number) => return Ok(PredicateLiteral::Number(number)),
            None => {
                return Err(ExpressionError::TypeError(format!(
                    "Cannot convert {} to JSON number",
                    s
                )))
            }
        }
    }

    // If we get here, it's not a recognized literal
    Err(ExpressionError::TypeError(format!(
        "Invalid literal: {}",
        s
    )))
}

// Compare two JSON values based on the comparison operator
fn compare_values(
    left: &Value,
    op: &ComparisonOperator,
    right: &PredicateRight<'_>,
) -> Result<bool> {
    match op {
        ComparisonOperator::Equal => Ok(values_equal(left, right)),
        ComparisonOperator::NotEqual => Ok(!values_equal(left, right)),

        // Handle numeric comparisons
        ComparisonOperator::GreaterThan => compare_ordered(left, right, |a, b| a > b),
        ComparisonOperator::LessThan => compare_ordered(left, right, |a, b| a < b),
        ComparisonOperator::GreaterThanOrEqual => compare_ordered(left, right, |a, b| a >= b),
        ComparisonOperator::LessThanOrEqual => compare_ordered(left, right, |a, b| a <= b),
        ComparisonOperator::Unspecified => Err(ExpressionError::Evaluation(
            "Unspecified comparison operator".to_string(),
        )),
    }
}

fn values_equal(left: &Value, right: &PredicateRight<'_>) -> bool {
    match right {
        PredicateRight::Resolved(value) => left == value.as_ref(),
        PredicateRight::Literal(PredicateLiteral::String(value)) => left.as_str() == Some(*value),
        PredicateRight::Literal(PredicateLiteral::Number(value)) => left.as_number() == Some(value),
        PredicateRight::Literal(PredicateLiteral::Bool(value)) => left.as_bool() == Some(*value),
    }
}

// Helper for ordered comparisons
fn compare_ordered<F>(left: &Value, right: &PredicateRight<'_>, compare_fn: F) -> Result<bool>
where
    F: Fn(&f64, &f64) -> bool,
{
    // Extract numeric values
    let left_num = extract_number(left)?;
    let right_num = extract_right_number(right)?;

    Ok(compare_fn(&left_num, &right_num))
}

fn extract_right_number(right: &PredicateRight<'_>) -> Result<f64> {
    match right {
        PredicateRight::Resolved(value) => extract_number(value),
        PredicateRight::Literal(PredicateLiteral::String(value)) => {
            value.parse::<f64>().map_err(|_| {
                ExpressionError::TypeError(format!("Cannot convert string to number: {value}"))
            })
        }
        PredicateRight::Literal(PredicateLiteral::Number(value)) => {
            value.as_f64().ok_or_else(|| {
                ExpressionError::TypeError(format!("Cannot convert number to f64: {value:?}"))
            })
        }
        PredicateRight::Literal(PredicateLiteral::Bool(value)) => {
            Ok(if *value { 1.0 } else { 0.0 })
        }
    }
}

// Extract a numeric value from a JSON value
fn extract_number(val: &Value) -> Result<f64> {
    match val {
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                Ok(f)
            } else {
                Err(ExpressionError::TypeError(format!(
                    "Cannot convert number to f64: {:?}",
                    n
                )))
            }
        }
        Value::String(s) => match s.parse::<f64>() {
            Ok(num) => Ok(num),
            Err(_) => Err(ExpressionError::TypeError(format!(
                "Cannot convert string to number: {}",
                s
            ))),
        },
        Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
        _ => Err(ExpressionError::TypeError(format!(
            "Cannot convert to number: {:?}",
            val
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;
    use webui_protocol::{ComparisonOperator, ConditionExpr, LogicalOperator};
    use webui_test_utils::test_json;

    #[test]
    fn test_simple_identifier() {
        // Test true identifier
        let condition = ConditionExpr::identifier("flag");

        let state = test_json!({
            "flag": true
        });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(true)),
            "Expected Ok(true), got {:?}",
            result
        );

        // Test false identifier
        let condition = ConditionExpr::identifier("flag");

        let state = test_json!({
            "flag": false
        });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(false)),
            "Expected Ok(false), got {:?}",
            result
        );

        // Test non-boolean identifier treated as boolean
        let condition = ConditionExpr::identifier("name");

        let state = test_json!({
            "name": "John"
        });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(true)),
            "Expected Ok(true), got {:?}",
            result
        ); // Non-empty string is truthy
    }

    #[test]
    fn test_predicate() {
        let condition = ConditionExpr::predicate("age", ComparisonOperator::GreaterThan, "18");

        // Test with age > 18
        let state = test_json!({
            "age": 21
        });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(true)),
            "Expected Ok(true), got {:?}",
            result
        );

        // Test with age < 18
        let state = test_json!({
            "age": 16
        });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(false)),
            "Expected Ok(false), got {:?}",
            result
        );
    }

    #[test]
    fn test_not_expression() {
        let condition = ConditionExpr::negated(ConditionExpr::identifier("flag"));

        // Test with flag = true
        let state = test_json!({
            "flag": true
        });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(false)),
            "Expected Ok(false), got {:?}",
            result
        );

        // Test with flag = false
        let state = test_json!({
            "flag": false
        });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(true)),
            "Expected Ok(true), got {:?}",
            result
        );
    }

    #[test]
    fn test_compound_expression() {
        // Test AND
        let condition = ConditionExpr::compound(
            ConditionExpr::identifier("isAdmin"),
            LogicalOperator::And,
            ConditionExpr::identifier("isActive"),
        );

        let state = test_json!({
            "isAdmin": true,
            "isActive": true
        });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(true)),
            "Expected Ok(true), got {:?}",
            result
        );

        // Test OR
        let condition = ConditionExpr::compound(
            ConditionExpr::identifier("isAdmin"),
            LogicalOperator::Or,
            ConditionExpr::identifier("isEditor"),
        );

        let state = test_json!({
            "isAdmin": false,
            "isEditor": true
        });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(true)),
            "Expected Ok(true), got {:?}",
            result
        );
    }

    #[test]
    fn test_comparison_operators() {
        let state = test_json!({
            "value": 10
        });

        // Test each comparison operator
        let ops = [
            (ComparisonOperator::GreaterThan, "5", true),
            (ComparisonOperator::GreaterThan, "15", false),
            (ComparisonOperator::LessThan, "15", true),
            (ComparisonOperator::LessThan, "5", false),
            (ComparisonOperator::Equal, "10", true),
            (ComparisonOperator::Equal, "11", false),
            (ComparisonOperator::NotEqual, "11", true),
            (ComparisonOperator::NotEqual, "10", false),
            (ComparisonOperator::GreaterThanOrEqual, "10", true),
            (ComparisonOperator::GreaterThanOrEqual, "11", false),
            (ComparisonOperator::LessThanOrEqual, "10", true),
            (ComparisonOperator::LessThanOrEqual, "9", false),
        ];

        for (op, right, expected) in ops.iter() {
            let condition = ConditionExpr::predicate("value", *op, *right);

            let result = evaluate(&condition, &state);
            assert!(
                matches!(result, Ok(val) if val == *expected),
                "Failed for operator {:?} with right value {}: expected Ok({}), got {:?}",
                op,
                right,
                expected,
                result
            );
        }
    }

    #[test]
    fn test_string_comparison() {
        let state = test_json!({
            "name": "John"
        });

        // Test string equality
        let condition = ConditionExpr::predicate("name", ComparisonOperator::Equal, "\"John\"");

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(true)),
            "Expected Ok(true), got {:?}",
            result
        );

        // Test string inequality
        let condition = ConditionExpr::predicate("name", ComparisonOperator::NotEqual, "\"Jane\"");

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(true)),
            "Expected Ok(true), got {:?}",
            result
        );
    }

    #[test]
    fn test_dotted_path() {
        let state = test_json!({
            "user": {
                "profile": {
                    "age": 25,
                    "name": "John"
                }
            }
        });

        // Test nested property access
        let condition =
            ConditionExpr::predicate("user.profile.age", ComparisonOperator::GreaterThan, "18");

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(true)),
            "Expected Ok(true), got {:?}",
            result
        );
    }

    #[test]
    fn test_borrowed_resolver_value() {
        let condition = ConditionExpr::identifier("flag");
        let value = Value::Bool(true);

        let result = evaluate_with_resolver(&condition, |path| {
            if path == "flag" {
                Some(Cow::Borrowed(&value))
            } else {
                None
            }
        });

        assert!(matches!(result, Ok(true)));
    }

    #[test]
    fn test_missing_value() {
        let state = test_json!({
            "user": {
                "name": "John"
            }
        });

        // Test with a missing value
        let condition = ConditionExpr::predicate("user.age", ComparisonOperator::GreaterThan, "18");

        let result = evaluate(&condition, &state);
        assert!(matches!(result, Err(ExpressionError::MissingValue(_))));
    }

    // === Identifier Edge Cases ===

    #[test]
    fn test_missing_identifier_is_falsy_before_negation() {
        let condition = ConditionExpr::identifier("notExist");
        let negated = ConditionExpr::negated(condition.clone());
        let state = test_json!({ "flag": true });

        assert!(
            matches!(evaluate(&condition, &state), Ok(false)),
            "a missing identifier must be a falsy operand"
        );
        assert!(
            matches!(evaluate(&negated, &state), Ok(true)),
            "negating a missing identifier must evaluate to true"
        );
    }

    #[test]
    fn test_zero_field() {
        let condition = ConditionExpr::identifier("zero");
        let state = test_json!({ "zero": 0 });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(false)),
            "Expected Ok(false), got {:?}",
            result
        );
    }

    #[test]
    fn test_empty_string_field() {
        let condition = ConditionExpr::identifier("empty");
        let state = test_json!({ "empty": "" });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(false)),
            "Expected Ok(false), got {:?}",
            result
        );
    }

    #[test]
    fn test_nonempty_array() {
        let condition = ConditionExpr::identifier("myList");
        let state = test_json!({ "myList": [1, 2, 3] });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(true)),
            "Expected Ok(true), got {:?}",
            result
        );
    }

    #[test]
    fn test_empty_array() {
        let condition = ConditionExpr::identifier("emptyList");
        let state = test_json!({ "emptyList": [] });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(false)),
            "Expected Ok(false), got {:?}",
            result
        );
    }

    // === Deep Dotted Path ===

    #[test]
    fn test_dotted_path_deep() {
        let condition = ConditionExpr::identifier("outer.nested.deep.value");
        let state = test_json!({
            "outer": { "nested": { "deep": { "value": true } } }
        });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(true)),
            "Expected Ok(true), got {:?}",
            result
        );
    }

    // === Comparison Edge Cases ===

    #[test]
    fn test_string_eq() {
        let condition =
            ConditionExpr::predicate("appearance", ComparisonOperator::Equal, "\"hub\"");
        let state = test_json!({ "appearance": "hub" });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(true)),
            "Expected Ok(true), got {:?}",
            result
        );
    }

    #[test]
    fn test_string_ne() {
        let condition =
            ConditionExpr::predicate("appearance", ComparisonOperator::NotEqual, "\"full-page\"");
        let state = test_json!({ "appearance": "hub" });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(true)),
            "Expected Ok(true), got {:?}",
            result
        );
    }

    #[test]
    fn test_number_eq() {
        let condition = ConditionExpr::predicate("x", ComparisonOperator::Equal, "5");
        let state = test_json!({ "x": 5 });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(true)),
            "Expected Ok(true), got {:?}",
            result
        );
    }

    #[test]
    fn test_less_than() {
        let condition = ConditionExpr::predicate("x", ComparisonOperator::LessThan, "y");
        let state = test_json!({ "x": 5, "y": 10 });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(true)),
            "Expected Ok(true), got {:?}",
            result
        );
    }

    #[test]
    fn test_greater_than() {
        let condition = ConditionExpr::predicate("y", ComparisonOperator::GreaterThan, "x");
        let state = test_json!({ "x": 5, "y": 10 });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(true)),
            "Expected Ok(true), got {:?}",
            result
        );
    }

    #[test]
    fn test_less_equal() {
        let condition = ConditionExpr::predicate("x", ComparisonOperator::LessThanOrEqual, "5");
        let state = test_json!({ "x": 5 });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(true)),
            "Expected Ok(true), got {:?}",
            result
        );
    }

    #[test]
    fn test_greater_equal() {
        let condition = ConditionExpr::predicate("y", ComparisonOperator::GreaterThanOrEqual, "10");
        let state = test_json!({ "y": 10 });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(true)),
            "Expected Ok(true), got {:?}",
            result
        );
    }

    #[test]
    fn test_nested_eq() {
        let condition = ConditionExpr::predicate("outer.inner", ComparisonOperator::Equal, "42");
        let state = test_json!({ "outer": { "inner": 42 } });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(true)),
            "Expected Ok(true), got {:?}",
            result
        );
    }

    // === Short-Circuit Evaluation ===

    #[test]
    fn test_and_true_true() {
        let condition = ConditionExpr::compound(
            ConditionExpr::identifier("isEnabled"),
            LogicalOperator::And,
            ConditionExpr::identifier("x"),
        );
        let state = test_json!({ "isEnabled": true, "x": 5 });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(true)),
            "Expected Ok(true), got {:?}",
            result
        );
    }

    #[test]
    fn test_and_true_false() {
        let condition = ConditionExpr::compound(
            ConditionExpr::identifier("isEnabled"),
            LogicalOperator::And,
            ConditionExpr::identifier("isDisabled"),
        );
        let state = test_json!({ "isEnabled": true, "isDisabled": false });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(false)),
            "Expected Ok(false), got {:?}",
            result
        );
    }

    #[test]
    fn test_or_false_false() {
        let condition = ConditionExpr::compound(
            ConditionExpr::identifier("isDisabled"),
            LogicalOperator::Or,
            ConditionExpr::identifier("zero"),
        );
        let state = test_json!({ "isDisabled": false, "zero": 0 });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(false)),
            "Expected Ok(false), got {:?}",
            result
        );
    }

    #[test]
    fn test_or_false_true() {
        let condition = ConditionExpr::compound(
            ConditionExpr::identifier("isDisabled"),
            LogicalOperator::Or,
            ConditionExpr::identifier("isEnabled"),
        );
        let state = test_json!({ "isDisabled": false, "isEnabled": true });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(true)),
            "Expected Ok(true), got {:?}",
            result
        );
    }

    // === Complex Expressions ===

    #[test]
    fn test_appearance_and_actions() {
        let condition = ConditionExpr::compound(
            ConditionExpr::predicate("appearance", ComparisonOperator::Equal, "\"hub\""),
            LogicalOperator::And,
            ConditionExpr::identifier("actions.trailing"),
        );
        let state = test_json!({
            "appearance": "hub",
            "actions": { "trailing": true }
        });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(true)),
            "Expected Ok(true), got {:?}",
            result
        );
    }

    #[test]
    fn test_negated_binary() {
        let condition = ConditionExpr::negated(ConditionExpr::predicate(
            "appearance",
            ComparisonOperator::Equal,
            "\"hub\"",
        ));
        let state = test_json!({ "appearance": "sidepanel" });

        let result = evaluate(&condition, &state);
        assert!(
            matches!(result, Ok(true)),
            "Expected Ok(true), got {:?}",
            result
        );
    }

    // === Single-term fast path ===

    #[test]
    fn test_single_term_matches_operator_guard_semantics() {
        let state = test_json!({
            "flag": true,
            "off": false,
            "count": 0,
            "name": "Alice",
            "user": { "age": 25 }
        });

        // Single identifiers and predicates carry no logical operators, so they
        // must evaluate normally and never surface an operator-limit error.
        let cases: [(ConditionExpr, bool); 6] = [
            (ConditionExpr::identifier("flag"), true),
            (ConditionExpr::identifier("off"), false),
            (ConditionExpr::identifier("count"), false),
            (ConditionExpr::identifier("name"), true),
            (
                ConditionExpr::predicate("user.age", ComparisonOperator::GreaterThan, "18"),
                true,
            ),
            (
                ConditionExpr::predicate("name", ComparisonOperator::Equal, "'Bob'"),
                false,
            ),
        ];

        for (condition, expected) in cases {
            assert_eq!(
                evaluate(&condition, &state).ok(),
                Some(expected),
                "single-term condition {:?} must evaluate to {}",
                condition,
                expected
            );
        }
    }

    #[test]
    fn test_single_term_predicate_still_reports_missing_value() {
        let state = test_json!({ "user": { "name": "John" } });
        let condition = ConditionExpr::predicate("user.age", ComparisonOperator::LessThan, "18");

        assert!(matches!(
            evaluate(&condition, &state),
            Err(ExpressionError::MissingValue(_))
        ));
    }

    #[test]
    fn test_deep_negation_uses_overflow_stack_without_recursion() {
        let mut condition = ConditionExpr::identifier("flag");
        for _ in 0..64 {
            condition = ConditionExpr::negated(condition);
        }
        let state = test_json!({ "flag": true });

        assert!(matches!(evaluate(&condition, &state), Ok(true)));
    }

    #[test]
    fn test_negation_depth_across_inline_stack_boundary() {
        // The inline stack holds INLINE_EXPRESSION_STACK slots before spilling
        // to the overflow Vec. Walk across that seam one level at a time so a
        // regression in the spill logic cannot hide between the tested depths.
        for depth in 0..=(INLINE_EXPRESSION_STACK + 2) {
            let mut condition = ConditionExpr::identifier("flag");
            for _ in 0..depth {
                condition = ConditionExpr::negated(condition);
            }
            let state = test_json!({ "flag": true });

            // Each negation flips the result, so parity is the oracle.
            let expected = depth % 2 == 0;
            assert_eq!(
                evaluate(&condition, &state).ok(),
                Some(expected),
                "negation depth {} must evaluate to {}",
                depth,
                expected
            );
        }
    }

    #[test]
    fn test_compound_tree_spills_to_overflow_stack() {
        // A compound node pushes two continuations per level, so a right-leaning
        // chain spills past the inline slots. This tree deliberately exceeds
        // MAX_LOGICAL_OPERATORS: `evaluate` no longer validates structure, so
        // the spill path must stay correct for callers that skip `validate`.
        let depth = INLINE_EXPRESSION_STACK;
        let mut condition = ConditionExpr::identifier("leaf0");
        for i in 1..=depth {
            condition = ConditionExpr::compound(
                ConditionExpr::identifier(format!("leaf{}", i)),
                LogicalOperator::And,
                condition,
            );
        }

        let mut all_true = serde_json::Map::new();
        for i in 0..=depth {
            all_true.insert(format!("leaf{}", i), Value::Bool(true));
        }
        let state = Value::Object(all_true.clone());
        assert_eq!(evaluate(&condition, &state).ok(), Some(true));

        // Flip the deepest leaf: the result must propagate back through every
        // spilled continuation rather than being lost at the boundary.
        let mut one_false = all_true;
        one_false.insert("leaf0".to_string(), Value::Bool(false));
        let state = Value::Object(one_false);
        assert_eq!(evaluate(&condition, &state).ok(), Some(false));
    }

    #[test]
    fn test_incomplete_quoted_literal_returns_error() {
        let condition = ConditionExpr::predicate("name", ComparisonOperator::Equal, "'");
        let state = test_json!({ "name": "" });

        assert!(matches!(
            evaluate(&condition, &state),
            Err(ExpressionError::TypeError(_))
        ));
    }

    #[test]
    fn test_single_term_fast_path_uses_same_resolver_contract() {
        let value = Value::String("active".to_string());
        let lookups = std::cell::Cell::new(0usize);

        let condition = ConditionExpr::predicate("status", ComparisonOperator::Equal, "'active'");
        let result = evaluate_with_resolver(&condition, |path| {
            lookups.set(lookups.get() + 1);
            if path == "status" {
                Some(Cow::Borrowed(&value))
            } else {
                None
            }
        });

        assert!(matches!(result, Ok(true)));
        assert_eq!(
            lookups.get(),
            1,
            "a single-term predicate must resolve exactly one path"
        );
    }
}
