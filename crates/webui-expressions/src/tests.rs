use super::*;
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
fn test_mixed_operators_error() {
    // Create a condition with mixed operators (AND and OR)
    let condition = ConditionExpr::compound(
        ConditionExpr::compound(
            ConditionExpr::identifier("a"),
            LogicalOperator::And,
            ConditionExpr::identifier("b"),
        ),
        LogicalOperator::Or,
        ConditionExpr::identifier("c"),
    );

    let state = test_json!({
        "a": true, "b": true, "c": true
    });

    let result = evaluate(&condition, &state);
    assert!(matches!(result, Err(ExpressionError::MixedOperators)));
}

#[test]
fn test_too_many_operators_error() {
    // Create a condition with more than 5 operators
    let mut condition = ConditionExpr::identifier("a");

    // Add 6 logical operators (exceeding the limit of 5)
    for i in 0..6 {
        condition = ConditionExpr::compound(
            condition,
            LogicalOperator::And,
            ConditionExpr::identifier(format!("var{}", i)),
        );
    }

    let state = test_json!({
        "a": true, "var0": true, "var1": true,
        "var2": true, "var3": true, "var4": true, "var5": true
    });

    let result = evaluate(&condition, &state);
    assert!(matches!(result, Err(ExpressionError::TooManyOperators(_))));
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
