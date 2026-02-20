//! WebUI Protocol implementation.
//!
//! This crate defines the protocol used by the WebUI framework for cross-platform
//! representation of UI components and templates. The protocol is serialized
//! using Protocol Buffers (protobuf) for optimal runtime performance.
use serde::Serialize;
use std::collections::HashMap;
use std::fmt;
use std::io;
use thiserror::Error;

pub mod protobuf;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Protocol validation error: {0}")]
    Validation(String),
}

pub type Result<T> = std::result::Result<T, ProtocolError>;

/// Logical operators for composing compound conditions.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub enum LogicalOperator {
    /// Represents a logical AND.
    And,
    /// Represents a logical OR.
    Or,
}

/// Operators used for comparing values in a predicate.
#[derive(Clone, Debug, Serialize, PartialEq)]
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
#[derive(Clone, Debug, Serialize, PartialEq)]
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

/// Defines the various types of fragments in the WebUI protocol.
/// Each variant specifies a different kind of UI operation:
/// - Raw contents,
/// - Components with additional styling,
/// - Loops over collections,
/// - Signal bindings for dynamic data,
/// - Conditional rendering.
#[derive(Clone, Debug, Serialize, PartialEq)]
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
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct WebUIFragmentRaw {
    /// The content to render.
    pub value: String,
}

/// A component fragment which includes CSS styling and references a nested fragment record.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct WebUIFragmentComponent {
    /// The identifier for the associated fragment record.
    #[serde(rename = "fragmentId")]
    pub fragment_id: String,
}

/// A loop (or "for") fragment that iterates over items in a collection.
#[derive(Clone, Debug, Serialize, PartialEq)]
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
#[derive(Clone, Debug, Serialize, PartialEq)]
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
#[derive(Clone, Debug, Serialize, PartialEq)]
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
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct WebUIProtocol {
    /// A map linking fragment identifiers to their associated fragments.
    pub fragments: WebUIFragmentRecords,
}

impl WebUIProtocol {
    // Helper method to validate and return the protocol
    fn validate_protocol(protocol: Self) -> Result<Self> {
        let fragments = &protocol.fragments;

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

    /// Serialize protocol to pretty JSON (for debug/inspect output only).
    pub fn to_json_pretty(&self) -> std::result::Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}
