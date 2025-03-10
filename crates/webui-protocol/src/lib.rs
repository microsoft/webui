//! WebUI Protocol implementation.
//!
//! This crate defines the protocol used by the WebUI framework for cross-platform
//! representation of UI components and templates.
use serde_json::Value;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

/// Represents a condition expression that can be used in an `if` stream.
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
            },
            ConditionExpr::Not(expr) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", "not")?;
                map.serialize_entry("condition", &*expr)?;
                map.end()
            },
            ConditionExpr::Compound { left, op, right } => {
                let mut map = serializer.serialize_map(Some(4))?;
                map.serialize_entry("kind", "compound")?;
                map.serialize_entry("left", &**left)?;
                map.serialize_entry("op", op)?;
                map.serialize_entry("right", &**right)?;
                map.end()
            },
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
                let left_val = helper.left.ok_or_else(|| serde::de::Error::missing_field("left"))?;
                let right_val = helper.right.ok_or_else(|| serde::de::Error::missing_field("right"))?;
                let left = match left_val {
                    Value::String(s) => s,
                    _ => return Err(serde::de::Error::custom("predicate left must be a string")),
                };
                let right = match right_val {
                    Value::String(s) => s,
                    _ => return Err(serde::de::Error::custom("predicate right must be a string")),
                };
                let operator = helper.operator.ok_or_else(|| serde::de::Error::missing_field("operator"))?;
                Ok(ConditionExpr::Predicate(Predicate { left, operator, right }))
            },
            "not" => {
                let condition = helper.condition.ok_or_else(|| serde::de::Error::missing_field("condition"))?;
                Ok(ConditionExpr::Not(condition))
            },
            "compound" => {
                // For compound, left and right may be either a shorthand string or a full ConditionExpr object.
                let left_field = helper.left.ok_or_else(|| serde::de::Error::missing_field("left"))?;
                let left: ConditionExpr = match left_field {
                    Value::String(s) => ConditionExpr::Identifier { value: s },
                    other => serde_json::from_value(other).map_err(serde::de::Error::custom)?,
                };
                let op = helper.op.ok_or_else(|| serde::de::Error::missing_field("op"))?;
                let right_field = helper.right.ok_or_else(|| serde::de::Error::missing_field("right"))?;
                let right: ConditionExpr = match right_field {
                    Value::String(s) => ConditionExpr::Identifier { value: s },
                    other => serde_json::from_value(other).map_err(serde::de::Error::custom)?,
                };
                Ok(ConditionExpr::Compound { left: Box::new(left), op, right: Box::new(right) })
            },
            "identifier" => {
                let value = helper.value.ok_or_else(|| serde::de::Error::missing_field("value"))?;
                Ok(ConditionExpr::Identifier { value })
            },
            _ => Err(serde::de::Error::unknown_variant(&helper.kind, 
                &["predicate", "not", "compound", "identifier"]))
        }
    }
}

/// Defines the various types of streams in the WebUI protocol.
/// Each variant specifies a different kind of UI operation:
/// - Raw contents,
/// - Components with additional styling,
/// - Loops over collections,
/// - Signal bindings for dynamic data,
/// - Conditional rendering.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum WebUIStream {
    /// Outputs static content.
    Raw(WebUIStreamRaw),
    /// A reusable component with styling.
    Component(WebUIStreamComponent),
    /// Iterates over a collection to generate repeated content.
    For(WebUIStreamFor),
    /// Connects dynamic content via signals.
    Signal(WebUIStreamSignal),
    /// Renders content conditionally.
    If(WebUIStreamIf),
}

/// A raw stream containing static text or HTML content.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WebUIStreamRaw {
    /// The content to render.
    pub value: String,
}

/// A component stream which includes CSS styling and references a nested stream record.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WebUIStreamComponent {
    /// CSS styling for the component.
    pub css: String,
    /// The identifier for the associated stream record.
    #[serde(rename = "streamId")]
    pub stream_id: String,
}

/// A loop (or "for") stream that iterates over items in a collection.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WebUIStreamFor {
    /// The name representing a singular item (e.g., "person").
    pub item: String,
    /// The collection name (e.g., "people").
    pub collection: String,
    /// The identifier for the stream to render for each item.
    #[serde(rename = "streamId")]
    pub stream_id: String,
}

/// A signal stream used for real-time or dynamic data binding.
/// The `raw` property indicates whether the signal value is rendered directly.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WebUIStreamSignal {
    /// The value or identifier of the signal.
    pub value: String,
    /// Determines if the value should be rendered as raw content.
    /// Defaults to false if not specified.
    #[serde(default)]
    pub raw: bool,
}

/// A conditional stream that evaluates a condition before rendering its content.
/// If the provided condition is met, the content identified by `streamId` is rendered.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WebUIStreamIf {
    /// The condition expression to evaluate.
    pub condition: ConditionExpr,
    /// The identifier for the stream record to render if the condition evaluates to true.
    #[serde(rename = "streamId")]
    pub stream_id: String,
}

/// A mapping of unique stream identifiers to their corresponding stream vectors.
/// This facilitates organizing the different parts of a webpage.
pub type WebUIStreamRecords = HashMap<String, Vec<WebUIStream>>;

/// The root protocol structure that represents the complete configuration for a webpage.
/// It contains all the stream records.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WebUIProtocol {
    /// A map linking stream identifiers to their associated streams.
    pub streams: WebUIStreamRecords,
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
        let streams = &protocol.streams;
        
        // Use an iterator-based approach to check for valid references
        // This avoids multiple hash lookups for each stream
        let invalid_ref = streams.iter().find_map(|(_, stream_vec)| {
            stream_vec.iter().find_map(|stream| {
                match stream {
                    WebUIStream::Component(component) if !streams.contains_key(&component.stream_id) => {
                        Some(ProtocolError::Validation(format!(
                            "Component references non-existent stream ID: {}", 
                            component.stream_id
                        )))
                    }
                    WebUIStream::For(for_loop) if !streams.contains_key(&for_loop.stream_id) => {
                        Some(ProtocolError::Validation(format!(
                            "For loop references non-existent stream ID: {}", 
                            for_loop.stream_id
                        )))
                    }
                    WebUIStream::If(if_cond) if !streams.contains_key(&if_cond.stream_id) => {
                        Some(ProtocolError::Validation(format!(
                            "If condition references non-existent stream ID: {}", 
                            if_cond.stream_id
                        )))
                    }
                    _ => None,
                }
            })
        });
        
        if let Some(err) = invalid_ref {
            return Err(err);
        }
        
        Ok(protocol)
    }
    
    // Keep the existing validate method for backward compatibility
    // and direct validation of instances
    pub fn validate(&self) -> Result<()> {
        // Convert self to owned, validate, then discard the result
        // This avoids duplicating the validation logic
        Self::validate_protocol(self.clone())?;
        Ok(())
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
            "streams": {
                "index.html": [
                    {
                        "type": "raw",
                        "value": "Hello, WebUI!\n"
                    },
                    {
                        "type": "for",
                        "item": "person",
                        "collection": "people",
                        "streamId": "for-1"
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
        
        let protocol = WebUIProtocol::from_json(json).unwrap();
        assert_eq!(protocol.streams.len(), 2);
        assert!(protocol.streams.contains_key("index.html"));
        assert!(protocol.streams.contains_key("for-1"));
    }

    #[test]
    fn test_invalid_reference() {
        let json = r#"{
            "streams": {
                "index.html": [
                    {
                        "type": "for",
                        "item": "person",
                        "collection": "people",
                        "streamId": "non-existent"
                    }
                ]
            }
        }"#;
        
        let result = WebUIProtocol::from_json(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_all_stream_types() {
        let json = r#"{
            "streams": {
                "index.html": [
                    { "type": "raw", "value": "Raw Content" },
                    { "type": "component", "css": ".my-style", "streamId": "component-1" },
                    { "type": "for", "item": "item", "collection": "items", "streamId": "for-1" },
                    { "type": "signal", "value": "user.name", "raw": true },
                    { "type": "if", "condition": {"kind": "identifier", "value": "isLoggedIn"}, "streamId": "if-1" }
                ],
                "component-1": [{ "type": "raw", "value": "Component Content" }],
                "for-1": [{ "type": "raw", "value": "Item Content" }],
                "if-1": [{ "type": "raw", "value": "Conditional Content" }]
            }
        }"#;
        
        let protocol = WebUIProtocol::from_json(json).unwrap();
        let streams = &protocol.streams["index.html"];
        
        assert_eq!(streams.len(), 5);
        
        match &streams[0] {
            WebUIStream::Raw(raw) => assert_eq!(raw.value, "Raw Content"),
            _ => panic!("Expected raw stream"),
        }
        
        match &streams[1] {
            WebUIStream::Component(component) => {
                assert_eq!(component.css, ".my-style");
                assert_eq!(component.stream_id, "component-1");
            },
            _ => panic!("Expected component stream"),
        }
        
        match &streams[2] {
            WebUIStream::For(for_loop) => {
                assert_eq!(for_loop.item, "item");
                assert_eq!(for_loop.collection, "items");
                assert_eq!(for_loop.stream_id, "for-1");
            },
            _ => panic!("Expected for stream"),
        }
        
        match &streams[3] {
            WebUIStream::Signal(signal) => {
                assert_eq!(signal.value, "user.name");
                assert_eq!(signal.raw, true);
            },
            _ => panic!("Expected signal stream"),
        }
        
        match &streams[4] {
            WebUIStream::If(if_cond) => {
                match &if_cond.condition {
                    ConditionExpr::Identifier { value } => assert_eq!(value, "isLoggedIn"),
                    _ => panic!("Expected identifier condition"),
                }
                assert_eq!(if_cond.stream_id, "if-1");
            },
            _ => panic!("Expected if stream"),
        }
    }

    #[test]
    fn test_condition_expressions() {
        // Create condition expressions programmatically
        
        // Test Identifier condition
        let identifier = ConditionExpr::Identifier { 
            value: "isAdmin".to_string() 
        };
        
        // Serialize and deserialize through Value
        let json_value = serde_json::to_value(&identifier).unwrap();
        let roundtrip: ConditionExpr = serde_json::from_value(json_value).unwrap();
        
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
        
        let json_value = serde_json::to_value(&predicate).unwrap();
        let roundtrip: ConditionExpr = serde_json::from_value(json_value).unwrap();
        
        match &roundtrip {
            ConditionExpr::Predicate(pred) => {
                assert_eq!(pred.left, "user.age");
                assert_eq!(pred.operator, ComparisonOperator::GreaterThan);
                assert_eq!(pred.right, "18");
            },
            _ => panic!("Expected predicate condition"),
        }
        
        // Test Not condition
        let not = ConditionExpr::Not(Box::new(ConditionExpr::Identifier {
            value: "isBlocked".to_string(),
        }));
        
        let json_value = serde_json::to_value(&not).unwrap();
        let roundtrip: ConditionExpr = serde_json::from_value(json_value).unwrap();
        
        match &roundtrip {
            ConditionExpr::Not(expr) => {
                match &**expr {
                    ConditionExpr::Identifier { value } => assert_eq!(value, "isBlocked"),
                    _ => panic!("Expected identifier inside Not condition"),
                }
            },
            _ => panic!("Expected Not condition"),
        }
        
        // Test Compound condition
        let compound = ConditionExpr::Compound {
            left: Box::new(ConditionExpr::Identifier { value: "isAdmin".to_string() }),
            op: LogicalOperator::Or,
            right: Box::new(ConditionExpr::Identifier { value: "isEditor".to_string() }),
        };
        
        let json_value = serde_json::to_value(&compound).unwrap();
        let roundtrip: ConditionExpr = serde_json::from_value(json_value).unwrap();
        
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
            },
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
            right: Box::new(ConditionExpr::Not(Box::new(ConditionExpr::Predicate(Predicate {
                left: "user.disabled".to_string(),
                operator: ComparisonOperator::Equal,
                right: "true".to_string(),
            })))),
        };
        
        // Reserialize and deserialize to test roundtrip
        let serialized = serde_json::to_value(&nested).unwrap();
        let deserialized: ConditionExpr = serde_json::from_value(serialized).unwrap();
        
        assert_eq!(nested, deserialized);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let protocol = WebUIProtocol {
            streams: {
                let mut map = HashMap::new();
                map.insert("main".to_string(), vec![
                    WebUIStream::Raw(WebUIStreamRaw { 
                        value: "Hello".to_string() 
                    }),
                    WebUIStream::If(WebUIStreamIf {
                        condition: ConditionExpr::Predicate(Predicate {
                            left: "user.logged_in".to_string(),
                            operator: ComparisonOperator::Equal,
                            right: "true".to_string(),
                        }),
                        stream_id: "welcome".to_string(),
                    }),
                ]);
                map.insert("welcome".to_string(), vec![
                    WebUIStream::Signal(WebUIStreamSignal {
                        value: "user.name".to_string(),
                        raw: false,
                    }),
                ]);
                map
            }
        };
        
        // Test to_json and from_json roundtrip
        let json = protocol.to_json().unwrap();
        let decoded = WebUIProtocol::from_json(&json).unwrap();
        assert_eq!(protocol, decoded);
        
        // Test to_json_pretty and from_json roundtrip
        let pretty_json = protocol.to_json_pretty().unwrap();
        let decoded_pretty = WebUIProtocol::from_json(&pretty_json).unwrap();
        assert_eq!(protocol, decoded_pretty);
    }

    #[test]
    fn test_validation_errors() {
        // Test missing stream reference in component
        let invalid_component = r#"{
            "streams": {
                "main": [
                    {
                        "type": "component",
                        "css": ".my-component",
                        "streamId": "missing-component"
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
        
        // Test missing stream reference in for loop
        let invalid_for = r#"{
            "streams": {
                "main": [
                    {
                        "type": "for",
                        "item": "item",
                        "collection": "items",
                        "streamId": "missing-for"
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
        
        // Test missing stream reference in if condition
        let invalid_if = r#"{
            "streams": {
                "main": [
                    {
                        "type": "if",
                        "condition": {"kind": "identifier", "value": "isLoggedIn"},
                        "streamId": "missing-if"
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
