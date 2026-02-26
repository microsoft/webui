use super::*;
use webui_protocol::condition_expr::Expr;

#[test]
fn test_parse_identifier() {
    let parser = ConditionParser::new();
    let result = parser
        .parse("isLoggedIn")
        .expect("Failed to parse identifier");

    assert!(matches!(&result.expr, Some(Expr::Identifier(id)) if id.value == "isLoggedIn"));

    // Test dot notation
    let result = parser
        .parse("user.isAdmin")
        .expect("Failed to parse dotted identifier");
    assert!(matches!(&result.expr, Some(Expr::Identifier(id)) if id.value == "user.isAdmin"));
}

#[test]
fn test_parse_not() {
    let parser = ConditionParser::new();
    let result = parser
        .parse("!isLoggedIn")
        .expect("Failed to parse NOT expression");

    // Check outer is Not
    assert!(matches!(&result.expr, Some(Expr::Not(not)) if
        matches!(not.condition.as_ref().and_then(|c| c.expr.as_ref()), Some(Expr::Identifier(id)) if id.value == "isLoggedIn")
    ));

    // Test with whitespace
    let result = parser
        .parse("! isLoggedIn")
        .expect("Failed to parse NOT expression with space");
    assert!(matches!(&result.expr, Some(Expr::Not(_))));
}

#[test]
fn test_parse_predicate() {
    let parser = ConditionParser::new();

    // Test each comparison operator
    let test_cases = vec![
        ("age > 18", ComparisonOperator::GreaterThan),
        ("age < 65", ComparisonOperator::LessThan),
        ("name == 'John'", ComparisonOperator::Equal),
        ("status != 'inactive'", ComparisonOperator::NotEqual),
        ("score >= 90", ComparisonOperator::GreaterThanOrEqual),
        ("score <= 100", ComparisonOperator::LessThanOrEqual),
    ];

    for (input, expected_op) in test_cases {
        let result = parser
            .parse(input)
            .expect("Failed to parse predicate expression");
        assert!(
            matches!(&result.expr, Some(Expr::Predicate(pred)) if ComparisonOperator::try_from(pred.operator) == Ok(expected_op))
        );
    }

    // Test string values with quotes
    let result = parser
        .parse("name == \"John\"")
        .expect("Failed to parse string comparison");
    assert!(matches!(&result.expr, Some(Expr::Predicate(pred)) if
        pred.left == "name" &&
        ComparisonOperator::try_from(pred.operator) == Ok(ComparisonOperator::Equal) &&
        pred.right == "John"
    ));
}

#[test]
fn test_parse_compound() {
    let parser = ConditionParser::new();

    // Test AND condition
    let result = parser
        .parse("isLoggedIn && age > 18")
        .expect("Failed to parse compound AND expression");
    assert!(matches!(&result.expr, Some(Expr::Compound(compound)) if
        matches!(compound.left.as_ref().and_then(|l| l.expr.as_ref()), Some(Expr::Identifier(id)) if id.value == "isLoggedIn") &&
        LogicalOperator::try_from(compound.op) == Ok(LogicalOperator::And) &&
        matches!(compound.right.as_ref().and_then(|r| r.expr.as_ref()), Some(Expr::Predicate(_)))
    ));

    // Test OR condition with "or" keyword
    let result = parser
        .parse("status == 'premium' or isAdmin")
        .expect("Failed to parse compound OR expression");
    assert!(matches!(&result.expr, Some(Expr::Compound(compound)) if
        matches!(compound.left.as_ref().and_then(|l| l.expr.as_ref()), Some(Expr::Predicate(_))) &&
        LogicalOperator::try_from(compound.op) == Ok(LogicalOperator::Or) &&
        matches!(compound.right.as_ref().and_then(|r| r.expr.as_ref()), Some(Expr::Identifier(id)) if id.value == "isAdmin")
    ));
}

#[test]
fn test_complex_expressions() {
    let parser = ConditionParser::new();

    // Complex AND/OR expression
    let result = parser
        .parse("age > 18 && isVerified || isAdmin")
        .expect("Failed to parse complex expression");
    assert!(matches!(&result.expr, Some(Expr::Compound(_))));

    // Negated expression
    let result = parser
        .parse("!isVerified")
        .expect("Failed to parse NOT expression");
    assert!(matches!(&result.expr, Some(Expr::Not(_))));
}

#[test]
fn test_invalid_expressions() {
    let parser = ConditionParser::new();

    // Empty input
    assert!(parser.parse("").is_err());

    // Invalid operators
    assert!(parser.parse("age << 18").is_err());

    // Incomplete expressions
    assert!(parser.parse("age >").is_err());
    assert!(parser.parse("&& isAdmin").is_err());
}
