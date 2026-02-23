//! WebUI Protocol implementation.
//!
//! This crate defines the protocol used by the WebUI framework for cross-platform
//! representation of UI components and templates. Types are generated directly
//! from `proto/webui.proto` using prost for optimal runtime performance —
//! no conversion layer between domain types and protobuf types.

use prost::Message;
use std::collections::HashMap;
use std::fmt;
use std::io;
use thiserror::Error;

/// Generated protobuf types from `proto/webui.proto`.
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/webui.rs"));
}

// Re-export all generated types at the crate root.
pub use proto::*;

// Type aliases preserving the `WebUI` naming convention.
// prost generates `WebUi*` from the proto `WebUI*` messages.
pub type WebUIProtocol = WebUiProtocol;
pub type WebUIFragment = WebUiFragment;
pub type WebUIFragmentRaw = WebUiFragmentRaw;
pub type WebUIFragmentComponent = WebUiFragmentComponent;
pub type WebUIFragmentFor = WebUiFragmentFor;
pub type WebUIFragmentSignal = WebUiFragmentSignal;
pub type WebUIFragmentIf = WebUiFragmentIf;
pub type WebUIFragmentAttribute = WebUiFragmentAttribute;

/// A mapping of unique fragment identifiers to their corresponding fragment lists.
pub type WebUIFragmentRecords = HashMap<String, FragmentList>;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Protocol validation error: {0}")]
    Validation(String),
}

pub type Result<T> = std::result::Result<T, ProtocolError>;

// ── Display implementations ─────────────────────────────────────────────

impl fmt::Display for ComparisonOperator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComparisonOperator::GreaterThan => write!(f, ">"),
            ComparisonOperator::LessThan => write!(f, "<"),
            ComparisonOperator::Equal => write!(f, "=="),
            ComparisonOperator::NotEqual => write!(f, "!="),
            ComparisonOperator::GreaterThanOrEqual => write!(f, ">="),
            ComparisonOperator::LessThanOrEqual => write!(f, "<="),
            ComparisonOperator::Unspecified => write!(f, "?"),
        }
    }
}

impl fmt::Display for LogicalOperator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogicalOperator::And => write!(f, "&&"),
            LogicalOperator::Or => write!(f, "||"),
            LogicalOperator::Unspecified => write!(f, "?"),
        }
    }
}

impl fmt::Display for ConditionExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.expr {
            Some(condition_expr::Expr::Identifier(id)) => write!(f, "{}", id.value),
            Some(condition_expr::Expr::Predicate(pred)) => {
                let op = ComparisonOperator::try_from(pred.operator)
                    .unwrap_or(ComparisonOperator::Unspecified);
                write!(f, "{} {} {}", pred.left, op, pred.right)
            }
            Some(condition_expr::Expr::Not(not)) => match &not.condition {
                Some(inner) => write!(f, "!({})", inner),
                None => write!(f, "!(?)"),
            },
            Some(condition_expr::Expr::Compound(compound)) => {
                let op =
                    LogicalOperator::try_from(compound.op).unwrap_or(LogicalOperator::Unspecified);
                let left_str = compound
                    .left
                    .as_ref()
                    .map(|l| l.to_string())
                    .unwrap_or_else(|| "?".to_string());
                let right_str = compound
                    .right
                    .as_ref()
                    .map(|r| r.to_string())
                    .unwrap_or_else(|| "?".to_string());
                write!(f, "({} {} {})", left_str, op, right_str)
            }
            None => write!(f, "<empty>"),
        }
    }
}

// ── Convenience constructors ────────────────────────────────────────────

impl WebUiFragment {
    /// Create a raw (static content) fragment.
    pub fn raw(value: impl Into<String>) -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::Raw(WebUiFragmentRaw {
                value: value.into(),
            })),
        }
    }

    /// Create a component fragment.
    pub fn component(fragment_id: impl Into<String>) -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::Component(
                WebUiFragmentComponent {
                    fragment_id: fragment_id.into(),
                },
            )),
        }
    }

    /// Create a for-loop fragment.
    pub fn for_loop(
        item: impl Into<String>,
        collection: impl Into<String>,
        fragment_id: impl Into<String>,
    ) -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::ForLoop(WebUiFragmentFor {
                item: item.into(),
                collection: collection.into(),
                fragment_id: fragment_id.into(),
            })),
        }
    }

    /// Create a signal fragment.
    pub fn signal(value: impl Into<String>, raw: bool) -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::Signal(WebUiFragmentSignal {
                value: value.into(),
                raw,
            })),
        }
    }

    /// Create an if-condition fragment.
    pub fn if_cond(condition: ConditionExpr, fragment_id: impl Into<String>) -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::IfCond(WebUiFragmentIf {
                condition: Some(condition),
                fragment_id: fragment_id.into(),
            })),
        }
    }

    /// Create a simple dynamic attribute fragment (value is a single signal name).
    pub fn attribute(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::Attribute(
                WebUiFragmentAttribute {
                    name: name.into(),
                    value: value.into(),
                    ..Default::default()
                },
            )),
        }
    }

    /// Create a template attribute fragment (mixed static + dynamic content).
    pub fn attribute_template(name: impl Into<String>, template: impl Into<String>) -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::Attribute(
                WebUiFragmentAttribute {
                    name: name.into(),
                    template: template.into(),
                    ..Default::default()
                },
            )),
        }
    }

    /// Create a complex attribute fragment (:-prefixed).
    pub fn attribute_complex(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::Attribute(
                WebUiFragmentAttribute {
                    name: name.into(),
                    value: value.into(),
                    complex: true,
                    ..Default::default()
                },
            )),
        }
    }

    /// Create a boolean attribute fragment (?-prefixed) with a condition tree.
    pub fn attribute_boolean(name: impl Into<String>, condition_tree: ConditionExpr) -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::Attribute(
                WebUiFragmentAttribute {
                    name: name.into(),
                    condition_tree: Some(condition_tree),
                    ..Default::default()
                },
            )),
        }
    }
}

impl ConditionExpr {
    /// Create an identifier condition.
    pub fn identifier(value: impl Into<String>) -> Self {
        Self {
            expr: Some(condition_expr::Expr::Identifier(IdentifierCondition {
                value: value.into(),
            })),
        }
    }

    /// Create a predicate condition.
    pub fn predicate(
        left: impl Into<String>,
        operator: ComparisonOperator,
        right: impl Into<String>,
    ) -> Self {
        Self {
            expr: Some(condition_expr::Expr::Predicate(Predicate {
                left: left.into(),
                operator: operator as i32,
                right: right.into(),
            })),
        }
    }

    /// Create a negation condition.
    pub fn negated(inner: ConditionExpr) -> Self {
        Self {
            expr: Some(condition_expr::Expr::Not(Box::new(NotCondition {
                condition: Some(Box::new(inner)),
            }))),
        }
    }

    /// Create a compound condition.
    pub fn compound(left: ConditionExpr, op: LogicalOperator, right: ConditionExpr) -> Self {
        Self {
            expr: Some(condition_expr::Expr::Compound(Box::new(
                CompoundCondition {
                    left: Some(Box::new(left)),
                    op: op as i32,
                    right: Some(Box::new(right)),
                },
            ))),
        }
    }
}

// ── Serialization / deserialization / validation ────────────────────────

impl WebUiProtocol {
    /// Validate that all fragment references point to existing fragment IDs.
    fn validate_protocol(protocol: Self) -> Result<Self> {
        let fragments = &protocol.fragments;

        let invalid_ref = fragments.iter().find_map(|(_, fragment_list)| {
            fragment_list
                .fragments
                .iter()
                .find_map(|frag| match frag.fragment.as_ref() {
                    Some(web_ui_fragment::Fragment::Component(comp))
                        if !fragments.contains_key(&comp.fragment_id) =>
                    {
                        Some(ProtocolError::Validation(format!(
                            "Component references non-existent fragment ID: {}",
                            comp.fragment_id
                        )))
                    }
                    Some(web_ui_fragment::Fragment::ForLoop(fl))
                        if !fragments.contains_key(&fl.fragment_id) =>
                    {
                        Some(ProtocolError::Validation(format!(
                            "For loop references non-existent fragment ID: {}",
                            fl.fragment_id
                        )))
                    }
                    Some(web_ui_fragment::Fragment::IfCond(ic))
                        if !fragments.contains_key(&ic.fragment_id) =>
                    {
                        Some(ProtocolError::Validation(format!(
                            "If condition references non-existent fragment ID: {}",
                            ic.fragment_id
                        )))
                    }
                    Some(web_ui_fragment::Fragment::Attribute(attr))
                        if !attr.template.is_empty() && !fragments.contains_key(&attr.template) =>
                    {
                        Some(ProtocolError::Validation(format!(
                            "Attribute references non-existent template fragment ID: {}",
                            attr.template
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

    /// Serialize protocol to protobuf binary format.
    pub fn to_protobuf(&self) -> Result<Vec<u8>> {
        let len = self.encoded_len();
        let mut buf = Vec::with_capacity(len);
        self.encode(&mut buf)
            .map_err(|e| ProtocolError::Validation(format!("Protobuf encode error: {e}")))?;
        Ok(buf)
    }

    /// Deserialize protocol from protobuf binary bytes with validation.
    pub fn from_protobuf(bytes: &[u8]) -> Result<Self> {
        let protocol = Self::decode(bytes)
            .map_err(|e| ProtocolError::Validation(format!("Protobuf decode error: {e}")))?;
        Self::validate_protocol(protocol)
    }

    /// Read and deserialize a protobuf file with validation.
    pub fn from_protobuf_file<P: AsRef<std::path::Path>>(path: P) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        Self::from_protobuf(&bytes)
    }

    /// Write protocol to a protobuf file.
    pub fn to_protobuf_file<P: AsRef<std::path::Path>>(&self, path: P) -> Result<()> {
        let bytes = self.to_protobuf()?;
        std::fs::write(path, bytes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_protocol() -> WebUIProtocol {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Hello, WebUI!\n"),
                    WebUIFragment::for_loop("person", "people", "for-1"),
                    WebUIFragment::signal("description", true),
                    WebUIFragment::if_cond(ConditionExpr::identifier("contact"), "if-1"),
                ],
            },
        );
        fragments.insert(
            "for-1".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::signal("person.name", false)],
            },
        );
        fragments.insert(
            "if-1".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("contact-card")],
            },
        );
        fragments.insert(
            "contact-card".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Hello, "),
                    WebUIFragment::signal("name", false),
                ],
            },
        );
        WebUIProtocol { fragments }
    }

    #[test]
    fn test_protobuf_roundtrip() {
        let protocol = sample_protocol();
        let bytes = protocol.to_protobuf().expect("encode failed");
        let decoded = WebUIProtocol::from_protobuf(&bytes).expect("decode failed");
        assert_eq!(protocol, decoded);
    }

    #[test]
    fn test_protobuf_all_fragment_types() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "main".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("text"),
                    WebUIFragment::component("comp"),
                    WebUIFragment::for_loop("x", "xs", "loop"),
                    WebUIFragment::signal("sig", true),
                    WebUIFragment::if_cond(
                        ConditionExpr::predicate("a", ComparisonOperator::GreaterThan, "1"),
                        "cond",
                    ),
                ],
            },
        );
        fragments.insert(
            "comp".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("c")],
            },
        );
        fragments.insert(
            "loop".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("l")],
            },
        );
        fragments.insert(
            "cond".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("i")],
            },
        );

        let protocol = WebUIProtocol { fragments };
        let bytes = protocol.to_protobuf().unwrap();
        let decoded = WebUIProtocol::from_protobuf(&bytes).unwrap();
        assert_eq!(protocol, decoded);
    }

    #[test]
    fn test_protobuf_all_comparison_operators() {
        let ops = [
            ComparisonOperator::GreaterThan,
            ComparisonOperator::LessThan,
            ComparisonOperator::Equal,
            ComparisonOperator::NotEqual,
            ComparisonOperator::GreaterThanOrEqual,
            ComparisonOperator::LessThanOrEqual,
        ];
        for op in &ops {
            let mut fragments = HashMap::new();
            fragments.insert(
                "main".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::if_cond(
                        ConditionExpr::predicate("a", *op, "b"),
                        "then",
                    )],
                },
            );
            fragments.insert(
                "then".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("ok")],
                },
            );
            let p = WebUIProtocol { fragments };
            let bytes = p.to_protobuf().unwrap();
            let decoded = WebUIProtocol::from_protobuf(&bytes).unwrap();
            assert_eq!(p, decoded);
        }
    }

    #[test]
    fn test_protobuf_nested_conditions() {
        let nested = ConditionExpr::compound(
            ConditionExpr::predicate("user.role", ComparisonOperator::Equal, "admin"),
            LogicalOperator::And,
            ConditionExpr::negated(ConditionExpr::predicate(
                "user.disabled",
                ComparisonOperator::Equal,
                "true",
            )),
        );

        let mut fragments = HashMap::new();
        fragments.insert(
            "main".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(nested, "then")],
            },
        );
        fragments.insert(
            "then".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("ok")],
            },
        );
        let p = WebUIProtocol { fragments };
        let bytes = p.to_protobuf().unwrap();
        let decoded = WebUIProtocol::from_protobuf(&bytes).unwrap();
        assert_eq!(p, decoded);
    }

    #[test]
    fn test_protobuf_compound_or_condition() {
        let compound = ConditionExpr::compound(
            ConditionExpr::identifier("isAdmin"),
            LogicalOperator::Or,
            ConditionExpr::identifier("isEditor"),
        );

        let mut fragments = HashMap::new();
        fragments.insert(
            "main".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(compound, "body")],
            },
        );
        fragments.insert(
            "body".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("yes")],
            },
        );
        let p = WebUIProtocol { fragments };
        let bytes = p.to_protobuf().unwrap();
        let decoded = WebUIProtocol::from_protobuf(&bytes).unwrap();
        assert_eq!(p, decoded);
    }

    #[test]
    fn test_protobuf_invalid_bytes() {
        let result = WebUIProtocol::from_protobuf(&[0xFF, 0xFF, 0xFF]);
        assert!(result.is_err());
    }

    #[test]
    fn test_protobuf_empty_bytes() {
        let result = WebUIProtocol::from_protobuf(&[]);
        assert!(result.is_ok());
        assert!(result.unwrap().fragments.is_empty());
    }

    #[test]
    fn test_protobuf_file_roundtrip() {
        let protocol = sample_protocol();
        let dir = std::env::temp_dir().join("webui-proto-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.bin");

        protocol.to_protobuf_file(&path).unwrap();
        let decoded = WebUIProtocol::from_protobuf_file(&path).unwrap();
        assert_eq!(protocol, decoded);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_protobuf_validation_catches_missing_reference() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "main".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("does-not-exist")],
            },
        );

        let protocol = WebUIProtocol { fragments };
        let buf = protocol.to_protobuf().unwrap();

        let result = WebUIProtocol::from_protobuf(&buf);
        assert!(result.is_err());
    }

    #[test]
    fn test_protobuf_validation_catches_missing_for_reference() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "main".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop("item", "items", "missing-for")],
            },
        );

        let protocol = WebUIProtocol { fragments };
        let buf = protocol.to_protobuf().unwrap();

        let result = WebUIProtocol::from_protobuf(&buf);
        assert!(result.is_err());
        if let Err(ProtocolError::Validation(msg)) = result {
            assert!(msg.contains("missing-for"));
        }
    }

    #[test]
    fn test_protobuf_validation_catches_missing_if_reference() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "main".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(
                    ConditionExpr::identifier("flag"),
                    "missing-if",
                )],
            },
        );

        let protocol = WebUIProtocol { fragments };
        let buf = protocol.to_protobuf().unwrap();

        let result = WebUIProtocol::from_protobuf(&buf);
        assert!(result.is_err());
        if let Err(ProtocolError::Validation(msg)) = result {
            assert!(msg.contains("missing-if"));
        }
    }

    #[test]
    fn test_protobuf_signal_default_raw_false() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "main".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::signal("name", false)],
            },
        );
        let p = WebUIProtocol { fragments };
        let bytes = p.to_protobuf().unwrap();
        let decoded = WebUIProtocol::from_protobuf(&bytes).unwrap();
        let frag = &decoded.fragments["main"].fragments[0];
        match frag.fragment.as_ref() {
            Some(web_ui_fragment::Fragment::Signal(s)) => assert!(!s.raw),
            _ => panic!("expected signal"),
        }
    }

    #[test]
    fn test_protobuf_pre_allocated_buffer() {
        let protocol = sample_protocol();
        let bytes = protocol.to_protobuf().unwrap();
        assert_eq!(bytes.len(), protocol.encoded_len());
    }
}
