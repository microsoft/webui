use crate::find_value_by_dotted_path::find_value_by_dotted_path;
use lazy_static::lazy_static;
use regex::Regex;
use serde_json::Value;
use serde_json::Number;

const OPERATORS: [&str; 10] = ["&&", "||", "!", "==", ">", ">=", "<", "<=", "(", ")"];

lazy_static! {
    static ref RE: Regex = {
        let mut patterns: Vec<_> = OPERATORS
            .iter()
            .map(|&separator| regex::escape(separator))
            .collect();
        patterns.sort_by(|a, b| b.len().cmp(&a.len()));
        let pattern = patterns.join("|");
        Regex::new(&pattern).unwrap()
    };
}

// Parses an expression into a vector of strings. The strings are either operators or operands.
pub fn parse_expression(expression: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut last = 0;
    for mat in RE.find_iter(expression) {
        if last != mat.start() {
            result.push(expression[last..mat.start()].trim().to_string());
        }
        result.push(mat.as_str().to_string());
        last = mat.end();
    }
    if last < expression.len() {
        result.push(expression[last..].trim().to_string());
    }
    result
}

// Safely evaluates an expression using a state object. The expression is a string that can contain
// logical operators and dotted paths to properties in the state object.
pub fn safe_evaluate_expression(expression: &str, state: &Value) -> bool {
    let tokens: Vec<String> = parse_expression(expression);
    let mut tokens_ref: Vec<&str> = tokens.iter().map(AsRef::as_ref).collect();
    tokens_ref.reverse();  // Reverse the tokens to allow us to use the stack efficiently
    match evaluate(&mut tokens_ref, state) {
        Value::Bool(b) => b,
        _ => false,
    }
}

// Evaluates a vector of tokens into a Value. The tokens are either operators or operands.
// It uses recursion to evaluate nested expressions within parentheses.
fn evaluate(tokens: &mut Vec<&str>, state: &Value) -> Value {
    let mut current_value = None;
    let mut current_operator = None;

    while let Some(token) = tokens.pop() {
        match token {
            "(" => {
                let value = evaluate(tokens, state);
                current_value = Some(apply_operator(current_value, current_operator, value));
                current_operator = None;
            }
            ")" => break,
            "==" | "!=" | "<" | "<=" | ">" | ">=" | "&&" | "||" | "!" => {
                if current_operator.is_some() {
                    let value = evaluate(tokens, state);
                    current_value = Some(apply_operator(current_value, current_operator, value));
                }
                current_operator = Some(token);
            }
            _ => {
                let value = parse_value(token, state);
                current_value = Some(apply_operator(current_value, current_operator, value));
                current_operator = None;
            }
        }
    }

    if current_operator.is_some() {
        let value = evaluate(tokens, state);
        current_value = Some(apply_operator(current_value, current_operator, value));
    }

    let current_value = current_value.unwrap_or(Value::Null);
    return current_value;
}

fn value_to_bool(value: Option<Value>) -> bool {
    value.map_or(false, |v| {
        match v {
            Value::Bool(b) => b,
            Value::Number(n) => n.as_f64() != Some(0.0),
            Value::String(s) => !s.is_empty(),
            Value::Array(a) => !a.is_empty(),
            Value::Object(o) => !o.is_empty(),
            _ => false,
        }
    })
}

// Applies an operator to two values. The operator can be a logical or comparison operator.
// The values can be booleans, numbers, strings, arrays, or objects.
fn apply_operator(value1: Option<Value>, operator: Option<&str>, value2: Value) -> Value {
    match operator {
        Some("==") => match (value1, value2) {
            (Some(Value::Bool(b1)), Value::Bool(b2)) => Value::Bool(b1 == b2),
            (Some(Value::Number(n1)), Value::Number(n2)) => Value::Bool(n1.as_f64() == n2.as_f64()),
            (Some(Value::String(s1)), Value::String(s2)) => Value::Bool(s1 == s2),
            (Some(Value::Array(a1)), Value::Array(a2)) => Value::Bool(a1 == a2),
            (Some(Value::Object(o1)), Value::Object(o2)) => Value::Bool(o1 == o2),
            _ => Value::Bool(false),
        }
        Some("!=") => Value::Bool(value1 != Some(value2)),
        Some(">") => {
            if let (Some(Value::Number(n1)), Value::Number(n2)) = (value1, value2) {
                Value::Bool(n1.as_f64() > n2.as_f64())
            } else {
                Value::Bool(false)
            }
        }
        Some("<") => {
            if let (Some(Value::Number(n1)), Value::Number(n2)) = (value1, value2) {
                Value::Bool(n1.as_f64() < n2.as_f64())
            } else {
                Value::Bool(false)
            }
        }
        Some(">=") => {
            if let (Some(Value::Number(n1)), Value::Number(n2)) = (value1, value2) {
                Value::Bool(n1.as_f64() >= n2.as_f64())
            } else {
                Value::Bool(false)
            }
        }
        Some("<=") => {
            if let (Some(Value::Number(n1)), Value::Number(n2)) = (value1, value2) {
                Value::Bool(n1.as_f64() <= n2.as_f64())
            } else {
                Value::Bool(false)
            }
        }
        Some("&&") => {
            let b1 = value_to_bool(value1);
            let b2 = value_to_bool(Some(value2));
            Value::Bool(b1 && b2)
        }
        Some("||") => {
            let b1 = value_to_bool(value1);
            let b2 = value_to_bool(Some(value2));
            Value::Bool(b1 || b2)
        }
        Some("!") => {
            let b = value_to_bool(Some(value2));
            Value::Bool(!b)
        }
        _ => value2
    }
}

// Parses a token into a Value. 
fn parse_value(token: &str, state: &Value) -> Value {
    if let Ok(number) = token.parse::<f64>() {
        let json_number = Number::from_f64(number).unwrap();
        return Value::Number(json_number);
    }
    match token {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => {
            let path_value = find_value_by_dotted_path(token, state);
            match path_value {
                Some(value) => value.clone(),
                None => Value::Null,
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    struct TestCaseParse {
        expression: &'static str,
        expected: Vec<&'static str>,
    }

    #[test]
    fn test_parse_expression() {
        let test_cases = vec![
            TestCaseParse { expression: "a && b || c", expected: vec!["a", "&&", "b", "||", "c"] },
            TestCaseParse { expression: "d == e", expected: vec!["d", "==", "e"] },
            TestCaseParse { expression: "f > g", expected: vec!["f", ">", "g"] },
            TestCaseParse { expression: "k&&l", expected: vec!["k", "&&", "l"] },
            TestCaseParse { expression: "m  ||  n", expected: vec!["m", "||", "n"] },
            TestCaseParse { expression: "o > p && q <= r || s == t", expected: vec!["o", ">", "p", "&&", "q", "<=", "r", "||", "s", "==", "t"] },
        ];

        for test_case in test_cases {
            assert_eq!(parse_expression(test_case.expression), test_case.expected, "Failed on expression: {}", test_case.expression);
        }
    }

    struct TestCaseEval {
        expression: &'static str,
        expected: bool,
    }

    #[test]
    fn test_safe_evaluate_expression() {
        // Create a JSON object
        let data = serde_json::json!({
            "name": {
                "first": "John",
                "last": "Doe"
            },
            "favorite": {
                "categories": {
                    "movies": ["The Matrix", "The Godfather"],
                    "music": ["Jazz", "Blues"]
                }
            },
            "age": 30,
            "is_student": true
        });

        let test_cases = vec![
            TestCaseEval { expression: "name.first && age", expected: true },
            TestCaseEval { expression: "age == 30", expected: true },
            TestCaseEval { expression: "favorite.categories.music && favorite.categories.movies", expected: true },
            TestCaseEval { expression: "age > 10", expected: true },
            TestCaseEval { expression: "name && name.first", expected: true },
            TestCaseEval { expression: "is_student", expected: true },
            TestCaseEval { expression: "is_student && name.first", expected: true },
            TestCaseEval { expression: "!is_student || true", expected: true },
            TestCaseEval { expression: "name.first &&", expected: false },
            TestCaseEval { expression: "name.middle", expected: false },
            TestCaseEval { expression: "name.first && (age == 31)", expected: false },
            TestCaseEval { expression: "name.first && false", expected: false },
            TestCaseEval { expression: "\"a\" == \"b\"", expected: false },
            TestCaseEval { expression: "!is_student", expected: false },
            TestCaseEval { expression: "!is_student && name.first", expected: false },
        ];
        
        for test_case in test_cases {
            assert_eq!(safe_evaluate_expression(test_case.expression, &data), test_case.expected, "Failed on expression: {}", test_case.expression);
        }
    }
}
