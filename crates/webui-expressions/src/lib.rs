//! WebUI expression evaluation module.
//!
//! This module handles the evaluation of condition expressions in WebUI templates.

use serde_json::Value;
use thiserror::Error;
use webui_protocol::{
    condition_expr, ComparisonOperator, CompoundCondition, ConditionExpr, LogicalOperator,
    Predicate,
};
use webui_state::find_value_by_dotted_path;

/// Error types for expression evaluation.
#[derive(Debug, Error)]
pub enum ExpressionError {
    #[error("Evaluation error: {0}")]
    Evaluation(String),

    #[error("Missing value: {0}")]
    MissingValue(String),

    #[error("Type error: {0}")]
    TypeError(String),

    #[error("Mixed logical operators: Cannot mix AND and OR operators")]
    MixedOperators,

    #[error("Too many logical operators: Maximum of 5 allowed, found {0}")]
    TooManyOperators(usize),

    #[error("Value comparison error: {0}")]
    Comparison(String),
}

pub type Result<T> = std::result::Result<T, ExpressionError>;

/// Evaluate a condition expression with the given state
pub fn evaluate(condition: &ConditionExpr, state: &Value) -> Result<bool> {
    // Count and validate logical operators
    let (logical_op_count, has_mixed_ops) = count_logical_operators(condition);

    if logical_op_count > 5 {
        return Err(ExpressionError::TooManyOperators(logical_op_count));
    }

    if has_mixed_ops {
        return Err(ExpressionError::MixedOperators);
    }

    // Use iterative evaluation
    evaluate_expr(condition, state)
}

// Helper function to count logical operators and check if they're mixed
fn count_logical_operators(condition: &ConditionExpr) -> (usize, bool) {
    let mut count = 0;
    let mut last_op: Option<i32> = None;
    let mut has_mixed = false;

    // We need to use a stack to avoid recursion
    let mut stack = vec![condition];

    while let Some(expr) = stack.pop() {
        match &expr.expr {
            Some(condition_expr::Expr::Compound(compound)) => {
                count += 1;

                // Check if we're mixing operators
                if let Some(last) = last_op {
                    if last != compound.op {
                        has_mixed = true;
                    }
                } else {
                    last_op = Some(compound.op);
                }

                // Push sub-expressions to stack
                if let Some(right) = compound.right.as_ref() {
                    stack.push(right);
                }
                if let Some(left) = compound.left.as_ref() {
                    stack.push(left);
                }
            }
            Some(condition_expr::Expr::Not(not_cond)) => {
                if let Some(inner) = not_cond.condition.as_ref() {
                    stack.push(inner);
                }
            }
            _ => {} // Predicates and identifiers don't have logical operators
        }
    }

    (count, has_mixed)
}

// Iterative evaluation of expressions
fn evaluate_expr(condition: &ConditionExpr, state: &Value) -> Result<bool> {
    match &condition.expr {
        Some(condition_expr::Expr::Predicate(pred)) => evaluate_predicate(pred, state),
        Some(condition_expr::Expr::Not(not_cond)) => {
            let inner = not_cond.condition.as_ref().ok_or_else(|| {
                ExpressionError::Evaluation("Not condition missing inner expression".to_string())
            })?;
            let result = evaluate_expr(inner, state)?;
            Ok(!result)
        }
        Some(condition_expr::Expr::Compound(compound)) => evaluate_compound(compound, state),
        Some(condition_expr::Expr::Identifier(id)) => {
            // Look up the identifier in state
            if let Some(val) = find_value_by_dotted_path(&id.value, state) {
                match val {
                    Value::Bool(b) => Ok(b),
                    Value::Null => Ok(false),
                    Value::Number(n) => Ok(!(n.as_f64() == Some(0.0))),
                    Value::String(s) => Ok(!s.is_empty()),
                    Value::Array(a) => Ok(!a.is_empty()),
                    Value::Object(o) => Ok(!o.is_empty()),
                }
            } else {
                Err(ExpressionError::MissingValue(id.value.clone()))
            }
        }
        None => Err(ExpressionError::Evaluation(
            "Empty condition expression".to_string(),
        )),
    }
}

fn evaluate_compound(compound: &CompoundCondition, state: &Value) -> Result<bool> {
    let left = compound.left.as_ref().ok_or_else(|| {
        ExpressionError::Evaluation("Compound missing left expression".to_string())
    })?;
    let right = compound.right.as_ref().ok_or_else(|| {
        ExpressionError::Evaluation("Compound missing right expression".to_string())
    })?;

    let left_result = evaluate_expr(left, state)?;
    let op = LogicalOperator::try_from(compound.op).map_err(|_| {
        ExpressionError::Evaluation(format!("Invalid logical operator: {}", compound.op))
    })?;

    match op {
        LogicalOperator::And => {
            if !left_result {
                return Ok(false);
            }
            evaluate_expr(right, state)
        }
        LogicalOperator::Or => {
            if left_result {
                return Ok(true);
            }
            evaluate_expr(right, state)
        }
        LogicalOperator::Unspecified => Err(ExpressionError::Evaluation(
            "Unspecified logical operator".to_string(),
        )),
    }
}

// Evaluate a predicate (comparison)
fn evaluate_predicate(predicate: &Predicate, state: &Value) -> Result<bool> {
    // Get left and right values
    let left_val = match find_value_by_dotted_path(&predicate.left, state) {
        Some(val) => val,
        None => return Err(ExpressionError::MissingValue(predicate.left.clone())),
    };

    // The right side could be a literal value or a variable reference
    let right_val = if is_literal(&predicate.right) {
        parse_literal(&predicate.right)?
    } else {
        match find_value_by_dotted_path(&predicate.right, state) {
            Some(val) => val,
            None => return Err(ExpressionError::MissingValue(predicate.right.clone())),
        }
    };

    let op = ComparisonOperator::try_from(predicate.operator).map_err(|_| {
        ExpressionError::Evaluation(format!(
            "Invalid comparison operator: {}",
            predicate.operator
        ))
    })?;

    // Compare values based on operator
    compare_values(&left_val, &op, &right_val)
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

// Parse a literal string into a JSON value
fn parse_literal(s: &str) -> Result<Value> {
    // Handle quoted strings
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        let content = &s[1..s.len() - 1];
        return Ok(Value::String(content.to_string()));
    }

    // Handle booleans
    if s == "true" {
        return Ok(Value::Bool(true));
    }
    if s == "false" {
        return Ok(Value::Bool(false));
    }

    // Handle numbers
    if let Ok(num) = s.parse::<i64>() {
        return Ok(Value::Number(num.into()));
    }

    if let Ok(num) = s.parse::<f64>() {
        // Create a JSON number from f64, handling error if it's not representable
        match serde_json::Number::from_f64(num) {
            Some(n) => return Ok(Value::Number(n)),
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
fn compare_values(left: &Value, op: &ComparisonOperator, right: &Value) -> Result<bool> {
    match op {
        ComparisonOperator::Equal => Ok(left == right),
        ComparisonOperator::NotEqual => Ok(left != right),

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

// Helper for ordered comparisons
fn compare_ordered<F>(left: &Value, right: &Value, compare_fn: F) -> Result<bool>
where
    F: Fn(&f64, &f64) -> bool,
{
    // Extract numeric values
    let left_num = extract_number(left)?;
    let right_num = extract_number(right)?;

    Ok(compare_fn(&left_num, &right_num))
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
mod tests;
