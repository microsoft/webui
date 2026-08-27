// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Parser for WebUI condition expressions.
//!
//! This module handles parsing condition expressions used in if directives.
//! It uses a non-recursive approach for parsing expressions.

use crate::{ParserError, Result};
use webui_protocol::{ComparisonOperator, ConditionExpr, LogicalOperator};

/// Parser for WebUI condition expressions.
pub struct ConditionParser;

impl ConditionParser {
    /// Create a new condition parser.
    pub fn new() -> Self {
        Self
    }

    /// Parse a condition string into a ConditionExpr.
    pub fn parse(&self, input: &str) -> Result<ConditionExpr> {
        let expr = self.parse_inner(input)?;

        // Structural rules are invariant for a given tree, so they are enforced
        // here at build time instead of on every render.
        expr.validate_structure()
            .map_err(|error| ParserError::Parse(error.to_string()))?;

        Ok(expr)
    }

    /// Parse without structural validation, for recursive sub-expression parsing.
    fn parse_inner(&self, input: &str) -> Result<ConditionExpr> {
        // Trim whitespace
        let input = input.trim();

        if input.is_empty() {
            return Err(ParserError::Parse("Empty condition string".to_string()));
        }

        // Try to parse each type of expression, starting with compound expressions
        if let Ok(expr) = self.parse_compound_expr(input) {
            return Ok(expr);
        }

        if let Ok(expr) = self.parse_not_expr(input) {
            return Ok(expr);
        }

        // A malformed predicate is a hard error rather than a fall-through:
        // silently retrying it as an identifier would hide the real mistake.
        if let Some(expr) = self.parse_predicate(input)? {
            return Ok(expr);
        }

        if let Ok(expr) = self.parse_identifier(input) {
            return Ok(expr);
        }

        Err(ParserError::Parse(format!(
            "Failed to parse condition: '{}'",
            input
        )))
    }

    /// Parse a simple identifier.
    fn parse_identifier(&self, input: &str) -> Result<ConditionExpr> {
        // Check if the input is a simple identifier (variable name)
        if self.is_valid_identifier(input) {
            return Ok(ConditionExpr::identifier(input));
        }

        Err(ParserError::Parse(format!(
            "Invalid identifier value: '{}'",
            input
        )))
    }

    /// Check if a string is a valid identifier.
    fn is_valid_identifier(&self, input: &str) -> bool {
        if input.is_empty() {
            return false;
        }

        // First character must be a letter or underscore
        let first_char = match input.chars().next() {
            Some(c) => c,
            // This shouldn't happen since we check isEmpty above
            None => return false,
        };

        if !first_char.is_alphabetic() && first_char != '_' {
            return false;
        }

        // Rest can be alphanumeric, underscore, or dot (for dot notation).
        input
            .chars()
            .skip(1)
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
    }

    /// Parse a negation expression (starting with !).
    fn parse_not_expr(&self, input: &str) -> Result<ConditionExpr> {
        let input = input.trim();

        // Check if the input starts with a negation operator
        if let Some(expr_str) = input.strip_prefix('!') {
            let expr_str = expr_str.trim();

            // Parse the negated expression
            let expr = self.parse_inner(expr_str)?;
            return Ok(ConditionExpr::negated(expr));
        }

        Err(ParserError::Parse("Not a negation expression".to_string()))
    }

    /// Parse a predicate (comparison).
    ///
    /// Returns `Ok(None)` when the input simply is not a predicate, and `Err`
    /// when it *is* a predicate but carries a malformed operand. The two cases
    /// are distinct: the first falls through to the other expression parsers,
    /// while the second must fail the build rather than reach the evaluator.
    fn parse_predicate(&self, input: &str) -> Result<Option<ConditionExpr>> {
        let input = input.trim();

        // Check if the input contains a comparison operator.
        for op_str in &[">=", "<=", "==", "!=", ">", "<"] {
            if let Some(pos) = input.find(op_str) {
                // Verify this is a standalone operator by checking surrounding characters
                // If it's part of another sequence like << or >>, it's not a valid operator
                let is_standalone = (pos == 0 || !input[..pos].ends_with(&op_str[0..1]))
                    && (pos + op_str.len() == input.len()
                        || !input[pos + op_str.len()..].starts_with(&op_str[0..1]));

                if !is_standalone {
                    continue;
                }

                // Split by operator
                let left = input[..pos].trim();
                let right = input[pos + op_str.len()..].trim();

                // Check that both sides are valid
                if left.is_empty() || right.is_empty() {
                    continue;
                }

                // Reject malformed quoted literals at build time. Letting one
                // through would slice outside a character boundary during
                // evaluation, aborting the render process at request time.
                for operand in [left, right] {
                    if Self::has_malformed_quotes(operand) {
                        return Err(ParserError::Parse(format!(
                            "unbalanced quotes in condition operand: {operand:?}"
                        )));
                    }
                }

                // Convert operator string to enum variant.
                let operator = match *op_str {
                    ">=" => ComparisonOperator::GreaterThanOrEqual,
                    "<=" => ComparisonOperator::LessThanOrEqual,
                    "==" => ComparisonOperator::Equal,
                    "!=" => ComparisonOperator::NotEqual,
                    ">" => ComparisonOperator::GreaterThan,
                    "<" => ComparisonOperator::LessThan,
                    _ => return Err(ParserError::Parse("Unknown comparison".to_string())),
                };

                return Ok(Some(ConditionExpr::predicate(left, operator, right)));
            }
        }

        Ok(None)
    }

    /// Detect operands whose quoting is unbalanced or degenerate.
    ///
    /// A bare `'` both opens and closes at the same byte, and `'abc` never
    /// closes. Both used to be forwarded verbatim to the evaluator.
    fn has_malformed_quotes(operand: &str) -> bool {
        let bytes = operand.as_bytes();
        let (Some(&first), Some(&last)) = (bytes.first(), bytes.last()) else {
            return false;
        };

        let opens = first == b'"' || first == b'\'';
        let closes = last == b'"' || last == b'\'';

        match (opens, closes) {
            // A quoted literal needs two distinct delimiter bytes that match.
            (true, true) => bytes.len() < 2 || first != last,
            // Opened without closing, or closed without opening.
            (true, false) | (false, true) => true,
            (false, false) => false,
        }
    }

    /// Parse a compound expression (with logical operators AND/OR).
    fn parse_compound_expr(&self, input: &str) -> Result<ConditionExpr> {
        let input = input.trim();

        // Look for AND operator
        if let Some(parts) = self.split_by_logical_op(input, &["&&", "and"], LogicalOperator::And) {
            return Ok(parts);
        }

        // Look for OR operator
        if let Some(parts) = self.split_by_logical_op(input, &["||", "or"], LogicalOperator::Or) {
            return Ok(parts);
        }

        Err(ParserError::Parse("Not a compound expression".to_string()))
    }

    /// Split a string by a logical operator and create a compound expression.
    fn split_by_logical_op(
        &self,
        input: &str,
        operators: &[&str],
        op: LogicalOperator,
    ) -> Option<ConditionExpr> {
        // Track nesting level for quotes
        let mut quote_char: Option<char> = None;
        let chars: Vec<char> = input.chars().collect();

        // Find operator indices
        let mut operator_indices = Vec::new();
        let mut i = 0;

        while i < chars.len() {
            match chars[i] {
                '\'' | '"' => {
                    if quote_char.is_none() {
                        quote_char = Some(chars[i]);
                    } else if quote_char == Some(chars[i]) {
                        quote_char = None;
                    }
                }
                _ => {
                    // Only check for operators when not inside quotes
                    if quote_char.is_none() {
                        for op_str in operators {
                            if i + op_str.len() <= chars.len() {
                                let slice = chars[i..i + op_str.len()].iter().collect::<String>();

                                if slice == *op_str {
                                    // Check if it's an actual operator and not part of a word
                                    let is_standalone = (i == 0 || !chars[i - 1].is_alphanumeric())
                                        && (i + op_str.len() == chars.len()
                                            || !chars[i + op_str.len()].is_alphanumeric());

                                    if is_standalone {
                                        operator_indices.push((i, op_str.len()));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            i += 1;
        }

        // Process the last operator (assuming right-to-left associativity)
        if let Some((op_start, op_len)) = operator_indices.first() {
            // Extract left and right parts
            let left_str = input[..(*op_start)].trim();
            let right_str = input[(*op_start + *op_len)..].trim();

            if !left_str.is_empty() && !right_str.is_empty() {
                // Parse the two sides of the operator
                if let Ok(left) = self.parse_inner(left_str) {
                    if let Ok(right) = self.parse_inner(right_str) {
                        return Some(ConditionExpr::compound(left, op, right));
                    }
                }
            }
        }

        None
    }
}

impl Default for ConditionParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
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
            pred.right == "\"John\""
        ));
    }

    #[test]
    fn test_malformed_quoted_literal_fails_at_build_time() {
        let parser = ConditionParser::new();

        // A bare quote opens and closes at the same byte. Forwarding it to the
        // evaluator used to slice `&s[1..0]` and abort the render process.
        for input in [
            "name == '",
            "name == \"",
            "name == 'John",
            "name == John'",
            "name == \"John'",
            "' == name",
        ] {
            assert!(
                parser.parse(input).is_err(),
                "expected `{input}` to be rejected at build time"
            );
        }
    }

    #[test]
    fn test_well_formed_literals_still_parse() {
        let parser = ConditionParser::new();

        for input in [
            "name == 'John'",
            "name == \"John\"",
            "name == \"it's\"",
            "count > 0",
            "ratio >= 1.5",
            "flag == true",
            "left == right",
            "user.profile.name != 'anon'",
        ] {
            assert!(
                parser.parse(input).is_ok(),
                "expected `{input}` to parse cleanly"
            );
        }
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

        // Complex expression with a uniform operator.
        let result = parser
            .parse("age > 18 && isVerified && isAdmin")
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

    #[test]
    fn test_tokenize_simple_identifier() {
        let parser = ConditionParser::new();
        let result = parser
            .parse("isVisible")
            .expect("Failed to parse simple identifier");

        assert!(matches!(&result.expr, Some(Expr::Identifier(id)) if id.value == "isVisible"));
    }

    #[test]
    fn test_tokenize_dotted_path() {
        let parser = ConditionParser::new();
        let result = parser
            .parse("appearance")
            .expect("Failed to parse identifier");

        assert!(matches!(&result.expr, Some(Expr::Identifier(id)) if id.value == "appearance"));
    }

    #[test]
    fn test_tokenize_binary_expression() {
        let parser = ConditionParser::new();
        let result = parser
            .parse("x > 5")
            .expect("Failed to parse binary expression");

        assert!(matches!(&result.expr, Some(Expr::Predicate(pred)) if
            pred.left == "x" &&
            ComparisonOperator::try_from(pred.operator) == Ok(ComparisonOperator::GreaterThan) &&
            pred.right == "5"
        ));
    }

    #[test]
    fn test_tokenize_complex_and_dotted() {
        let parser = ConditionParser::new();
        let result = parser
            .parse("appearance == \"hub\" && actions.trailing")
            .expect("Failed to parse complex and-dotted expression");

        assert!(matches!(&result.expr, Some(Expr::Compound(compound)) if
            matches!(compound.left.as_ref().and_then(|l| l.expr.as_ref()), Some(Expr::Predicate(pred)) if
                pred.left == "appearance" &&
                ComparisonOperator::try_from(pred.operator) == Ok(ComparisonOperator::Equal) &&
                pred.right == "\"hub\""
            ) &&
            LogicalOperator::try_from(compound.op) == Ok(LogicalOperator::And) &&
            matches!(compound.right.as_ref().and_then(|r| r.expr.as_ref()), Some(Expr::Identifier(id)) if id.value == "actions.trailing")
        ));
    }

    #[test]
    fn test_tokenize_unary_not() {
        let parser = ConditionParser::new();
        let result = parser
            .parse("!disabled")
            .expect("Failed to parse unary not expression");

        assert!(matches!(&result.expr, Some(Expr::Not(not)) if
            matches!(not.condition.as_ref().and_then(|c| c.expr.as_ref()), Some(Expr::Identifier(id)) if id.value == "disabled")
        ));
    }

    #[test]
    fn test_tokenize_undefined_comparison() {
        let parser = ConditionParser::new();
        let result = parser
            .parse("foo == undefined")
            .expect("Failed to parse undefined comparison");

        assert!(matches!(&result.expr, Some(Expr::Predicate(pred)) if
            pred.left == "foo" &&
            ComparisonOperator::try_from(pred.operator) == Ok(ComparisonOperator::Equal) &&
            pred.right == "undefined"
        ));
    }

    #[test]
    fn test_reject_mixed_and_or() {
        // Mixing `&&` and `||` without explicit grouping is ambiguous, so it is
        // rejected while building rather than surfacing per render. This also
        // matches the NodeJS parser.
        let parser = ConditionParser::new();

        for input in ["a && b || c", "a || b && c", "!(a && b) || c"] {
            assert!(
                parser.parse(input).is_err(),
                "expected `{input}` to be rejected at build time"
            );
        }

        // Uniform operators of either kind remain valid.
        assert!(parser.parse("a && b && c").is_ok());
        assert!(parser.parse("a || b || c").is_ok());
    }

    #[test]
    fn test_reject_too_many_tokens() {
        // Chains longer than MAX_LOGICAL_OPERATORS are rejected while building.
        let parser = ConditionParser::new();
        assert!(parser
            .parse("a && b && c && d && e && f && g && h && i && j")
            .is_err());

        // A chain exactly at the limit still parses into nested Compound nodes.
        let result = parser
            .parse("a && b && c && d && e && f")
            .expect("chain at the operator limit must parse");
        assert!(matches!(&result.expr, Some(Expr::Compound(compound)) if
            matches!(compound.left.as_ref().and_then(|l| l.expr.as_ref()), Some(Expr::Identifier(id)) if id.value == "a") &&
            LogicalOperator::try_from(compound.op) == Ok(LogicalOperator::And) &&
            matches!(compound.right.as_ref().and_then(|r| r.expr.as_ref()), Some(Expr::Compound(_)))
        ));
    }
}
