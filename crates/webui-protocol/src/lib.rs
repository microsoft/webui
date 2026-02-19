//! WebUI Protocol implementation.
//!
//! This crate defines the protocol used by the WebUI framework for cross-platform
//! representation of UI components and templates.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("JSON parsing error: {0}")]
    JsonParse(#[from] serde_json::Error),

    #[error("Protocol validation error: {0}")]
    Validation(String),
}

pub type Result<T> = std::result::Result<T, ProtocolError>;

/// Logical operators for composing compound conditions.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum LogicalOperator {
    /// Represents a logical AND.
    And,
    /// Represents a logical OR.
    Or,
}

/// Operators used for comparing values in a predicate.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ComparisonOperator {
    /// Greater than operator ( > ).
    GreaterThan,
    /// Less than operator ( < ).
    LessThan,
    /// Equal to operator ( == ).
    Equal,
    /// Not equal to operator ( != ).
    NotEqual,
    /// Greater than or equal to operator ( >= ).
    GreaterThanOrEqual,
    /// Less than or equal to operator ( <= ).
    LessThanOrEqual,
}

/// A simple predicate that compares two values.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Predicate {
    /// The left-hand side value.
    pub left: String,
    /// The operator used in comparison.
    pub operator: ComparisonOperator,
    /// The right-hand side value.
    pub right: String,
}

/// Represents a condition expression that can be used in an `if` fragment.
/// The condition can be:
/// - A simple predicate,
/// - A negated condition,
/// - Or a compound condition combining two expressions.
#[derive(Clone, Debug, PartialEq)]
pub enum ConditionExpr {
    /// A simple predicate condition.
    Predicate(Predicate),
    /// A negation of a condition expression.
    Not(Box<ConditionExpr>),
    /// A compound condition combining two expressions using a logical operator.
    Compound {
        /// The left-hand side condition.
        left: Box<ConditionExpr>,
        /// The logical operator (And or Or).
        op: LogicalOperator,
        /// The right-hand side condition.
        right: Box<ConditionExpr>,
    },
    /// A identifier condition, single variable.
    Identifier {
        /// The identifier to evaluate.
        value: String,
    },
}

// Implement Display for ConditionExpr
impl fmt::Display for ConditionExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConditionExpr::Identifier { value } => {
                write!(f, "{}", value)
            }
            ConditionExpr::Predicate(pred) => {
                write!(f, "{} {} {}", pred.left, pred.operator, pred.right)
            }
            ConditionExpr::Not(expr) => {
                write!(f, "!({})", expr)
            }
            ConditionExpr::Compound { left, op, right } => {
                // Use parentheses for compound expressions to maintain precedence
                write!(f, "({} {} {})", left, op, right)
            }
        }
    }
}

// Implement Display for ComparisonOperator
impl fmt::Display for ComparisonOperator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComparisonOperator::GreaterThan => write!(f, ">"),
            ComparisonOperator::LessThan => write!(f, "<"),
            ComparisonOperator::Equal => write!(f, "=="),
            ComparisonOperator::NotEqual => write!(f, "!="),
            ComparisonOperator::GreaterThanOrEqual => write!(f, ">="),
            ComparisonOperator::LessThanOrEqual => write!(f, "<="),
        }
    }
}

// Implement Display for LogicalOperator
impl fmt::Display for LogicalOperator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogicalOperator::And => write!(f, "&&"),
            LogicalOperator::Or => write!(f, "||"),
        }
    }
}

// Custom serialization implementation for ConditionExpr
impl Serialize for ConditionExpr {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;

        match self {
            ConditionExpr::Predicate(pred) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", "predicate")?;
                map.serialize_entry("left", &pred.left)?;
                map.serialize_entry("operator", &pred.operator)?;
                map.serialize_entry("right", &pred.right)?;
                map.end()
            }
            ConditionExpr::Not(expr) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", "not")?;
                map.serialize_entry("condition", expr)?;
                map.end()
            }
            ConditionExpr::Compound { left, op, right } => {
                let mut map = serializer.serialize_map(Some(4))?;
                map.serialize_entry("kind", "compound")?;
                map.serialize_entry("left", &**left)?;
                map.serialize_entry("op", op)?;
                map.serialize_entry("right", &**right)?;
                map.end()
            }
            ConditionExpr::Identifier { value } => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", "identifier")?;
                map.serialize_entry("value", value)?;
                map.end()
            }
        }
    }
}

// Custom deserialization implementation for ConditionExpr

impl<'de> Deserialize<'de> for ConditionExpr {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Helper {
            kind: String,
            #[serde(default)]
            left: Option<Value>,
            #[serde(default)]
            operator: Option<ComparisonOperator>,
            #[serde(default)]
            right: Option<Value>,
            #[serde(default)]
            condition: Option<Box<ConditionExpr>>,
            #[serde(default)]
            op: Option<LogicalOperator>,
            #[serde(default)]
            value: Option<String>,
        }

        let helper = Helper::deserialize(deserializer)?;

        match helper.kind.as_str() {
            "predicate" => {
                // Expect left and right to be strings
                let left_val = helper
                    .left
                    .ok_or_else(|| serde::de::Error::missing_field("left"))?;
                let right_val = helper
                    .right
                    .ok_or_else(|| serde::de::Error::missing_field("right"))?;
                let left = match left_val {
                    Value::String(s) => s,
                    _ => return Err(serde::de::Error::custom("predicate left must be a string")),
                };
                let right = match right_val {
                    Value::String(s) => s,
                    _ => return Err(serde::de::Error::custom("predicate right must be a string")),
                };
                let operator = helper
                    .operator
                    .ok_or_else(|| serde::de::Error::missing_field("operator"))?;
                Ok(ConditionExpr::Predicate(Predicate {
                    left,
                    operator,
                    right,
                }))
            }
            "not" => {
                let condition = helper
                    .condition
                    .ok_or_else(|| serde::de::Error::missing_field("condition"))?;
                Ok(ConditionExpr::Not(condition))
            }
            "compound" => {
                // For compound, left and right may be either a shorthand string or a full ConditionExpr object.
                let left_field = helper
                    .left
                    .ok_or_else(|| serde::de::Error::missing_field("left"))?;
                let left: ConditionExpr = match left_field {
                    Value::String(s) => ConditionExpr::Identifier { value: s },
                    other => serde_json::from_value(other).map_err(serde::de::Error::custom)?,
                };
                let op = helper
                    .op
                    .ok_or_else(|| serde::de::Error::missing_field("op"))?;
                let right_field = helper
                    .right
                    .ok_or_else(|| serde::de::Error::missing_field("right"))?;
                let right: ConditionExpr = match right_field {
                    Value::String(s) => ConditionExpr::Identifier { value: s },
                    other => serde_json::from_value(other).map_err(serde::de::Error::custom)?,
                };
                Ok(ConditionExpr::Compound {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                })
            }
            "identifier" => {
                let value = helper
                    .value
                    .ok_or_else(|| serde::de::Error::missing_field("value"))?;
                Ok(ConditionExpr::Identifier { value })
            }
            _ => Err(serde::de::Error::unknown_variant(
                &helper.kind,
                &["predicate", "not", "compound", "identifier"],
            )),
        }
    }
}

/// Defines the various types of fragments in the WebUI protocol.
/// Each variant specifies a different kind of UI operation:
/// - Raw contents,
/// - Components with additional styling,
/// - Loops over collections,
/// - Signal bindings for dynamic data,
/// - Conditional rendering.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum WebUIFragment {
    /// Outputs static content.
    Raw(WebUIFragmentRaw),
    /// A reusable component with styling.
    Component(WebUIFragmentComponent),
    /// Iterates over a collection to generate repeated content.
    For(WebUIFragmentFor),
    /// Connects dynamic content via signals.
    Signal(WebUIFragmentSignal),
    /// Renders content conditionally.
    If(WebUIFragmentIf),
}

/// A raw fragment containing static text or HTML content.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WebUIFragmentRaw {
    /// The content to render.
    pub value: String,
}

/// A component fragment which includes CSS styling and references a nested fragment record.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WebUIFragmentComponent {
    /// The identifier for the associated fragment record.
    #[serde(rename = "fragmentId")]
    pub fragment_id: String,
}

/// A loop (or "for") fragment that iterates over items in a collection.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WebUIFragmentFor {
    /// The name representing a singular item (e.g., "person").
    pub item: String,
    /// The collection name (e.g., "people").
    pub collection: String,
    /// The identifier for the fragment to render for each item.
    #[serde(rename = "fragmentId")]
    pub fragment_id: String,
}

/// A signal fragment used for real-time or dynamic data binding.
/// The `raw` property indicates whether the signal value is rendered directly.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WebUIFragmentSignal {
    /// The value or identifier of the signal.
    pub value: String,
    /// Determines if the value should be rendered as raw content.
    /// Defaults to false if not specified.
    #[serde(default)]
    pub raw: bool,
}

/// A conditional fragment that evaluates a condition before rendering its content.
/// If the provided condition is met, the content identified by `fragmentId` is rendered.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WebUIFragmentIf {
    /// The condition expression to evaluate.
    pub condition: ConditionExpr,
    /// The identifier for the fragment record to render if the condition evaluates to true.
    #[serde(rename = "fragmentId")]
    pub fragment_id: String,
}

/// A mapping of unique fragment identifiers to their corresponding fragment vectors.
/// This facilitates organizing the different parts of a webpage.
pub type WebUIFragmentRecords = HashMap<String, Vec<WebUIFragment>>;

/// The root protocol structure that represents the complete configuration for a webpage.
/// It contains all the fragment records.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WebUIProtocol {
    /// A map linking fragment identifiers to their associated fragments.
    pub fragments: WebUIFragmentRecords,
}

impl WebUIProtocol {
    /// Parse WebUIProtocol from a JSON string
    pub fn from_json(json: &str) -> Result<Self> {
        let protocol = serde_json::from_str(json)?;
        Self::validate_protocol(protocol)
    }

    /// Parse WebUIProtocol from a JSON file
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        Self::from_reader(reader)
    }

    /// Parse WebUIProtocol from a reader
    pub fn from_reader<R: Read>(reader: R) -> Result<Self> {
        let protocol = serde_json::from_reader(reader)?;
        Self::validate_protocol(protocol)
    }

    // Helper method to validate and return the protocol
    fn validate_protocol(protocol: Self) -> Result<Self> {
        // Validation check
        let fragments = &protocol.fragments;

        // Use an iterator-based approach to check for valid references
        // This avoids multiple hash lookups for each fragment
        let invalid_ref = fragments.iter().find_map(|(_, fragment_vec)| {
            fragment_vec.iter().find_map(|fragment| match fragment {
                WebUIFragment::Component(component)
                    if !fragments.contains_key(&component.fragment_id) =>
                {
                    Some(ProtocolError::Validation(format!(
                        "Component references non-existent fragment ID: {}",
                        component.fragment_id
                    )))
                }
                WebUIFragment::For(for_loop) if !fragments.contains_key(&for_loop.fragment_id) => {
                    Some(ProtocolError::Validation(format!(
                        "For loop references non-existent fragment ID: {}",
                        for_loop.fragment_id
                    )))
                }
                WebUIFragment::If(if_cond) if !fragments.contains_key(&if_cond.fragment_id) => {
                    Some(ProtocolError::Validation(format!(
                        "If condition references non-existent fragment ID: {}",
                        if_cond.fragment_id
                    )))
                }
                _ => None,
            })
        });

        if let Some(err) = invalid_ref {
            return Err(err);
        }

        Ok(protocol)
    }

    /// Serialize protocol to JSON
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    /// Serialize protocol to pretty JSON
    pub fn to_json_pretty(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

// Implement TryFrom for more ergonomic conversions with built-in validation
impl TryFrom<&str> for WebUIProtocol {
    type Error = ProtocolError;

    fn try_from(json: &str) -> Result<Self> {
        Self::from_json(json)
    }
}

impl TryFrom<serde_json::Value> for WebUIProtocol {
    type Error = ProtocolError;

    fn try_from(value: serde_json::Value) -> Result<Self> {
        let protocol = serde_json::from_value(value)?;
        Self::validate_protocol(protocol)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_protocol() {
        let json = r#"{
            "fragments": {
                "index.html": [
                    {
                        "type": "raw",
                        "value": "Hello, WebUI!\n"
                    },
                    {
                        "type": "for",
                        "item": "person",
                        "collection": "people",
                        "fragmentId": "for-1"
                    }
                ],
                "for-1": [
                    {
                        "type": "signal",
                        "value": "person.name"
                    }
                ]
            }
        }"#;

        let protocol = WebUIProtocol::from_json(json).expect("Failed to parse valid protocol");
        assert_eq!(protocol.fragments.len(), 2);

        let index_fragment = &protocol.fragments["index.html"];
        assert_eq!(index_fragment.len(), 2);

        let raw_fragment = &index_fragment[0];
        assert!(matches!(raw_fragment, WebUIFragment::Raw(_)));
        if let WebUIFragment::Raw(raw) = raw_fragment {
            assert_eq!(raw.value, "Hello, WebUI!\n");
        } else {
            panic!("Expected raw fragment");
        }

        let for_fragment = &index_fragment[1];
        assert!(matches!(for_fragment, WebUIFragment::For(_)));
        if let WebUIFragment::For(for_loop) = for_fragment {
            assert_eq!(for_loop.item, "person");
            assert_eq!(for_loop.collection, "people");
            assert_eq!(for_loop.fragment_id, "for-1");
        } else {
            panic!("Expected signal fragment");
        }

        let for_fragment = &protocol.fragments["for-1"];
        assert_eq!(for_fragment.len(), 1);
        let signal_fragment = &for_fragment[0];
        assert!(matches!(signal_fragment, WebUIFragment::Signal(_)));
        if let WebUIFragment::Signal(signal) = signal_fragment {
            assert_eq!(signal.value, "person.name");
            assert!(!signal.raw);
        } else {
            panic!("Expected signal fragment");
        }
    }

    #[test]
    fn test_invalid_reference() {
        let json = r#"{
            "fragments": {
                "index.html": [
                    {
                        "type": "for",
                        "item": "person",
                        "collection": "people",
                        "fragmentId": "non-existent"
                    }
                ]
            }
        }"#;

        let result = WebUIProtocol::from_json(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_all_fragment_types() {
        let json = r#"{
            "fragments": {
                "index.html": [
                    { "type": "raw", "value": "Raw Content" },
                    { "type": "component", "fragmentId": "component-1" },
                    { "type": "for", "item": "item", "collection": "items", "fragmentId": "for-1" },
                    { "type": "signal", "value": "user.name", "raw": true },
                    { "type": "if", "condition": {"kind": "identifier", "value": "isLoggedIn"}, "fragmentId": "if-1" }
                ],
                "component-1": [{ "type": "raw", "value": "Component Content" }],
                "for-1": [{ "type": "raw", "value": "Item Content" }],
                "if-1": [{ "type": "raw", "value": "Conditional Content" }]
            }
        }"#;

        let protocol = WebUIProtocol::from_json(json)
            .expect("Failed to parse protocol with all fragment types");
        let fragments = &protocol.fragments["index.html"];

        assert_eq!(fragments.len(), 5);

        match &fragments[0] {
            WebUIFragment::Raw(raw) => assert_eq!(raw.value, "Raw Content"),
            _ => panic!("Expected raw fragment"),
        }

        match &fragments[1] {
            WebUIFragment::Component(component) => {
                assert_eq!(component.fragment_id, "component-1");
            }
            _ => panic!("Expected component fragment"),
        }

        match &fragments[2] {
            WebUIFragment::For(for_loop) => {
                assert_eq!(for_loop.item, "item");
                assert_eq!(for_loop.collection, "items");
                assert_eq!(for_loop.fragment_id, "for-1");
            }
            _ => panic!("Expected for fragment"),
        }

        match &fragments[3] {
            WebUIFragment::Signal(signal) => {
                assert_eq!(signal.value, "user.name");
                assert!(signal.raw);
            }
            _ => panic!("Expected signal fragment"),
        }

        match &fragments[4] {
            WebUIFragment::If(if_cond) => {
                match &if_cond.condition {
                    ConditionExpr::Identifier { value } => assert_eq!(value, "isLoggedIn"),
                    _ => panic!("Expected identifier condition"),
                }
                assert_eq!(if_cond.fragment_id, "if-1");
            }
            _ => panic!("Expected if fragment"),
        }
    }

    #[test]
    fn test_condition_expressions() {
        // Create condition expressions programmatically

        // Test Identifier condition
        let identifier = ConditionExpr::Identifier {
            value: "isAdmin".to_string(),
        };

        // Serialize and deserialize through Value
        let json_value = serde_json::to_value(&identifier).expect("Failed to serialize identifier");
        let roundtrip: ConditionExpr =
            serde_json::from_value(json_value).expect("Failed to deserialize identifier");

        match &roundtrip {
            ConditionExpr::Identifier { value } => assert_eq!(value, "isAdmin"),
            _ => panic!("Expected identifier condition"),
        }

        // Test Predicate condition
        let predicate = ConditionExpr::Predicate(Predicate {
            left: "user.age".to_string(),
            operator: ComparisonOperator::GreaterThan,
            right: "18".to_string(),
        });

        let json_value = serde_json::to_value(&predicate).expect("Failed to serialize predicate");
        let roundtrip: ConditionExpr =
            serde_json::from_value(json_value).expect("Failed to deserialize predicate");

        match &roundtrip {
            ConditionExpr::Predicate(pred) => {
                assert_eq!(pred.left, "user.age");
                assert_eq!(pred.operator, ComparisonOperator::GreaterThan);
                assert_eq!(pred.right, "18");
            }
            _ => panic!("Expected predicate condition"),
        }

        // Test Not condition
        let not = ConditionExpr::Not(Box::new(ConditionExpr::Identifier {
            value: "isBlocked".to_string(),
        }));

        let json_value = serde_json::to_value(&not).expect("Failed to serialize NOT expression");
        let roundtrip: ConditionExpr =
            serde_json::from_value(json_value).expect("Failed to deserialize NOT expression");

        match &roundtrip {
            ConditionExpr::Not(expr) => match &**expr {
                ConditionExpr::Identifier { value } => assert_eq!(value, "isBlocked"),
                _ => panic!("Expected identifier inside Not condition"),
            },
            _ => panic!("Expected Not condition"),
        }

        // Test Compound condition
        let compound = ConditionExpr::Compound {
            left: Box::new(ConditionExpr::Identifier {
                value: "isAdmin".to_string(),
            }),
            op: LogicalOperator::Or,
            right: Box::new(ConditionExpr::Identifier {
                value: "isEditor".to_string(),
            }),
        };

        let json_value =
            serde_json::to_value(&compound).expect("Failed to serialize compound expression");
        let roundtrip: ConditionExpr =
            serde_json::from_value(json_value).expect("Failed to deserialize compound expression");

        match &roundtrip {
            ConditionExpr::Compound { left, op, right } => {
                match &**left {
                    ConditionExpr::Identifier { value } => assert_eq!(value, "isAdmin"),
                    _ => panic!("Expected identifier for left condition"),
                }
                assert_eq!(op, &LogicalOperator::Or);
                match &**right {
                    ConditionExpr::Identifier { value } => assert_eq!(value, "isEditor"),
                    _ => panic!("Expected identifier for right condition"),
                }
            }
            _ => panic!("Expected Compound condition"),
        }
    }

    #[test]
    fn test_nested_conditions() {
        // Create a complex nested condition
        let nested = ConditionExpr::Compound {
            left: Box::new(ConditionExpr::Predicate(Predicate {
                left: "user.role".to_string(),
                operator: ComparisonOperator::Equal,
                right: "admin".to_string(),
            })),
            op: LogicalOperator::And,
            right: Box::new(ConditionExpr::Not(Box::new(ConditionExpr::Predicate(
                Predicate {
                    left: "user.disabled".to_string(),
                    operator: ComparisonOperator::Equal,
                    right: "true".to_string(),
                },
            )))),
        };

        // Reserialize and deserialize to test roundtrip
        let serialized =
            serde_json::to_value(&nested).expect("Failed to serialize nested condition");
        let deserialized: ConditionExpr =
            serde_json::from_value(serialized).expect("Failed to deserialize nested condition");

        assert_eq!(nested, deserialized);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let protocol = WebUIProtocol {
            fragments: {
                let mut map = HashMap::new();
                map.insert(
                    "main".to_string(),
                    vec![
                        WebUIFragment::Raw(WebUIFragmentRaw {
                            value: "Hello".to_string(),
                        }),
                        WebUIFragment::If(WebUIFragmentIf {
                            condition: ConditionExpr::Predicate(Predicate {
                                left: "user.logged_in".to_string(),
                                operator: ComparisonOperator::Equal,
                                right: "true".to_string(),
                            }),
                            fragment_id: "welcome".to_string(),
                        }),
                    ],
                );
                map.insert(
                    "welcome".to_string(),
                    vec![WebUIFragment::Signal(WebUIFragmentSignal {
                        value: "user.name".to_string(),
                        raw: false,
                    })],
                );
                map
            },
        };

        // Test to_json and from_json roundtrip
        let json = protocol
            .to_json()
            .expect("Failed to serialize protocol to JSON");
        let decoded =
            WebUIProtocol::from_json(&json).expect("Failed to deserialize protocol from JSON");
        assert_eq!(protocol, decoded);

        // Test to_json_pretty and from_json roundtrip
        let pretty_json = protocol
            .to_json_pretty()
            .expect("Failed to serialize protocol to pretty JSON");
        let decoded_pretty = WebUIProtocol::from_json(&pretty_json)
            .expect("Failed to deserialize protocol from pretty JSON");
        assert_eq!(protocol, decoded_pretty);
    }

    #[test]
    fn test_validation_errors() {
        // Test missing fragment reference in component
        let invalid_component = r#"{
            "fragments": {
                "main": [
                    {
                        "type": "component",
                        "fragmentId": "missing-component"
                    }
                ]
            }
        }"#;

        let result = WebUIProtocol::from_json(invalid_component);
        assert!(result.is_err());

        if let Err(ProtocolError::Validation(msg)) = result {
            assert!(msg.contains("missing-component"));
        } else {
            panic!("Expected validation error");
        }

        // Test missing fragment reference in for loop
        let invalid_for = r#"{
            "fragments": {
                "main": [
                    {
                        "type": "for",
                        "item": "item",
                        "collection": "items",
                        "fragmentId": "missing-for"
                    }
                ]
            }
        }"#;

        let result = WebUIProtocol::from_json(invalid_for);
        assert!(result.is_err());

        if let Err(ProtocolError::Validation(msg)) = result {
            assert!(msg.contains("missing-for"));
        } else {
            panic!("Expected validation error");
        }

        // Test missing fragment reference in if condition
        let invalid_if = r#"{
            "fragments": {
                "main": [
                    {
                        "type": "if",
                        "condition": {"kind": "identifier", "value": "isLoggedIn"},
                        "fragmentId": "missing-if"
                    }
                ]
            }
        }"#;

        let result = WebUIProtocol::from_json(invalid_if);
        assert!(result.is_err());

        if let Err(ProtocolError::Validation(msg)) = result {
            assert!(msg.contains("missing-if"));
        } else {
            panic!("Expected validation error");
        }
    }
}
