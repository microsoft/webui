//! WebUI expression evaluation module.
//!
//! This module handles the evaluation of condition expressions in WebUI templates.

use serde_json::Value;
use webui_protocol::{ConditionExpr, LogicalOperator, ComparisonOperator, Predicate};
use webui_state::find_value_by_dotted_path;
use thiserror::Error;

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
    let mut last_op: Option<LogicalOperator> = None;
    let mut has_mixed = false;
    
    // We need to use a stack to avoid recursion
    let mut stack = vec![condition];
    
    while let Some(expr) = stack.pop() {
        match expr {
            ConditionExpr::Compound { left, op, right } => {
                count += 1;
                
                // Check if we're mixing operators
                if let Some(last) = &last_op {
                    if last != op {
                        has_mixed = true;
                    }
                } else {
                    last_op = Some(op.clone());
                }
                
                // Push sub-expressions to stack
                stack.push(right);
                stack.push(left);
            },
            ConditionExpr::Not(inner) => {
                stack.push(inner);
            },
            _ => {} // Predicates and identifiers don't have logical operators
        }
    }
    
    (count, has_mixed)
}

// Iterative evaluation of expressions
fn evaluate_expr(condition: &ConditionExpr, state: &Value) -> Result<bool> {
    match condition {
        ConditionExpr::Predicate(pred) => evaluate_predicate(pred, state),
        ConditionExpr::Not(expr) => {
            let result = evaluate_expr(expr, state)?;
            Ok(!result)
        },
        ConditionExpr::Compound { left, op, right } => {
            let left_result = evaluate_expr(left, state)?;
            
            // Short-circuit evaluation
            match op {
                LogicalOperator::And => {
                    if !left_result {
                        return Ok(false);
                    }
                    evaluate_expr(right, state)
                },
                LogicalOperator::Or => {
                    if left_result {
                        return Ok(true);
                    }
                    evaluate_expr(right, state)
                }
            }
        },
        ConditionExpr::Identifier { value } => {
            // Look up the identifier in state
            if let Some(val) = find_value_by_dotted_path(value, state) {
                match val {
                    Value::Bool(b) => Ok(b),
                    Value::Null => Ok(false),
                    Value::Number(n) => Ok(!(n.as_f64() == Some(0.0))),
                    Value::String(s) => Ok(!s.is_empty()),
                    Value::Array(a) => Ok(!a.is_empty()),
                    Value::Object(o) => Ok(!o.is_empty()),
                }
            } else {
                Err(ExpressionError::MissingValue(value.clone()))
            }
        }
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
    
    // Compare values based on operator
    compare_values(&left_val, &predicate.operator, &right_val)
}

// Check if a string is a literal value
fn is_literal(s: &str) -> bool {
    // A string is a literal if:
    // - It starts with a quote (single or double)
    // - It's a number (starts with a digit or negative sign followed by a digit)
    // - It's a boolean ("true" or "false")
    s.starts_with('"') || s.starts_with('\'') || 
    s.starts_with(|c: char| c.is_ascii_digit() || (c == '-' && s.len() > 1 && s.chars().nth(1).is_some_and(|c| c.is_ascii_digit()))) ||
    s == "true" || s == "false"
}

// Parse a literal string into a JSON value
fn parse_literal(s: &str) -> Result<Value> {
    // Handle quoted strings
    if (s.starts_with('"') && s.ends_with('"')) || 
       (s.starts_with('\'') && s.ends_with('\'')) {
        let content = &s[1..s.len()-1];
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
            None => return Err(ExpressionError::TypeError(format!("Cannot convert {} to JSON number", s))),
        }
    }
    
    // If we get here, it's not a recognized literal
    Err(ExpressionError::TypeError(format!("Invalid literal: {}", s)))
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
    }
}

// Helper for ordered comparisons
fn compare_ordered<F>(left: &Value, right: &Value, compare_fn: F) -> Result<bool> 
where 
    F: Fn(&f64, &f64) -> bool
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
                Err(ExpressionError::TypeError(format!("Cannot convert number to f64: {:?}", n)))
            }
        },
        Value::String(s) => {
            match s.parse::<f64>() {
                Ok(num) => Ok(num),
                Err(_) => Err(ExpressionError::TypeError(format!("Cannot convert string to number: {}", s))),
            }
        },
        Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
        _ => Err(ExpressionError::TypeError(format!("Cannot convert to number: {:?}", val))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use webui_protocol::{Predicate, ComparisonOperator, LogicalOperator, ConditionExpr};
    
    #[test]
    fn test_simple_identifier() {
        // Test true identifier
        let condition = ConditionExpr::Identifier { 
            value: "flag".to_string() 
        };
        
        let state = json!({
            "flag": true
        });
        
        let result = evaluate(&condition, &state).unwrap();
        assert!(result);
        
        // Test false identifier
        let condition = ConditionExpr::Identifier { 
            value: "flag".to_string() 
        };
        
        let state = json!({
            "flag": false
        });
        
        let result = evaluate(&condition, &state).unwrap();
        assert!(!result);
        
        // Test non-boolean identifier treated as boolean
        let condition = ConditionExpr::Identifier { 
            value: "name".to_string() 
        };
        
        let state = json!({
            "name": "John"
        });
        
        let result = evaluate(&condition, &state).unwrap();
        assert!(result); // Non-empty string is truthy
    }
    
    #[test]
    fn test_predicate() {
        let condition = ConditionExpr::Predicate(Predicate {
            left: "age".to_string(),
            operator: ComparisonOperator::GreaterThan,
            right: "18".to_string(),
        });
        
        // Test with age > 18
        let state = json!({
            "age": 21
        });
        
        let result = evaluate(&condition, &state).unwrap();
        assert!(result);
        
        // Test with age < 18
        let state = json!({
            "age": 16
        });
        
        let result = evaluate(&condition, &state).unwrap();
        assert!(!result);
    }
    
    #[test]
    fn test_not_expression() {
        let condition = ConditionExpr::Not(Box::new(ConditionExpr::Identifier {
            value: "flag".to_string()
        }));
        
        // Test with flag = true
        let state = json!({
            "flag": true
        });
        
        let result = evaluate(&condition, &state).unwrap();
        assert!(!result);
        
        // Test with flag = false
        let state = json!({
            "flag": false
        });
        
        let result = evaluate(&condition, &state).unwrap();
        assert!(result);
    }
    
    #[test]
    fn test_compound_expression() {
        // Test AND
        let condition = ConditionExpr::Compound {
            left: Box::new(ConditionExpr::Identifier {
                value: "isAdmin".to_string()
            }),
            op: LogicalOperator::And,
            right: Box::new(ConditionExpr::Identifier {
                value: "isActive".to_string()
            }),
        };
        
        let state = json!({
            "isAdmin": true,
            "isActive": true
        });
        
        let result = evaluate(&condition, &state).unwrap();
        assert!(result);
        
        // Test OR
        let condition = ConditionExpr::Compound {
            left: Box::new(ConditionExpr::Identifier {
                value: "isAdmin".to_string()
            }),
            op: LogicalOperator::Or,
            right: Box::new(ConditionExpr::Identifier {
                value: "isEditor".to_string()
            }),
        };
        
        let state = json!({
            "isAdmin": false,
            "isEditor": true
        });
        
        let result = evaluate(&condition, &state).unwrap();
        assert!(result);
    }
    
    #[test]
    fn test_mixed_operators_error() {
        // Create a condition with mixed operators (AND and OR)
        let condition = ConditionExpr::Compound {
            left: Box::new(ConditionExpr::Compound {
                left: Box::new(ConditionExpr::Identifier {
                    value: "a".to_string()
                }),
                op: LogicalOperator::And,
                right: Box::new(ConditionExpr::Identifier {
                    value: "b".to_string()
                }),
            }),
            op: LogicalOperator::Or,
            right: Box::new(ConditionExpr::Identifier {
                value: "c".to_string()
            }),
        };
        
        let state = json!({
            "a": true, "b": true, "c": true
        });
        
        let result = evaluate(&condition, &state);
        assert!(matches!(result, Err(ExpressionError::MixedOperators)));
    }
    
    #[test]
    fn test_too_many_operators_error() {
        // Create a condition with more than 5 operators
        let mut condition = ConditionExpr::Identifier {
            value: "a".to_string()
        };
        
        // Add 6 logical operators (exceeding the limit of 5)
        for i in 0..6 {
            condition = ConditionExpr::Compound {
                left: Box::new(condition),
                op: LogicalOperator::And,
                right: Box::new(ConditionExpr::Identifier {
                    value: format!("var{}", i)
                }),
            };
        }
        
        let state = json!({
            "a": true, "var0": true, "var1": true, 
            "var2": true, "var3": true, "var4": true, "var5": true
        });
        
        let result = evaluate(&condition, &state);
        assert!(matches!(result, Err(ExpressionError::TooManyOperators(_))));
    }
    
    #[test]
    fn test_comparison_operators() {
        let state = json!({
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
            let condition = ConditionExpr::Predicate(Predicate {
                left: "value".to_string(),
                operator: op.clone(),
                right: right.to_string(),
            });
            
            let result = evaluate(&condition, &state).unwrap();
            assert_eq!(result, *expected, "Failed for operator {:?} with right value {}", op, right);
        }
    }
    
    #[test]
    fn test_string_comparison() {
        let state = json!({
            "name": "John"
        });
        
        // Test string equality
        let condition = ConditionExpr::Predicate(Predicate {
            left: "name".to_string(),
            operator: ComparisonOperator::Equal,
            right: "\"John\"".to_string(),
        });
        
        let result = evaluate(&condition, &state).unwrap();
        assert!(result);
        
        // Test string inequality
        let condition = ConditionExpr::Predicate(Predicate {
            left: "name".to_string(),
            operator: ComparisonOperator::NotEqual,
            right: "\"Jane\"".to_string(),
        });
        
        let result = evaluate(&condition, &state).unwrap();
        assert!(result);
    }
    
    #[test]
    fn test_dotted_path() {
        let state = json!({
            "user": {
                "profile": {
                    "age": 25,
                    "name": "John"
                }
            }
        });
        
        // Test nested property access
        let condition = ConditionExpr::Predicate(Predicate {
            left: "user.profile.age".to_string(),
            operator: ComparisonOperator::GreaterThan,
            right: "18".to_string(),
        });
        
        let result = evaluate(&condition, &state).unwrap();
        assert!(result);
    }
    
    #[test]
    fn test_missing_value() {
        let state = json!({
            "user": {
                "name": "John"
            }
        });
        
        // Test with a missing value
        let condition = ConditionExpr::Predicate(Predicate {
            left: "user.age".to_string(),
            operator: ComparisonOperator::GreaterThan,
            right: "18".to_string(),
        });
        
        let result = evaluate(&condition, &state);
        assert!(matches!(result, Err(ExpressionError::MissingValue(_))));
    }
}
