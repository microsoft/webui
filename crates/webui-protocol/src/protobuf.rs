//! Protobuf serialization and deserialization for the WebUI protocol.
//!
//! Provides high-performance binary encoding via Protocol Buffers, optimized
//! for fast runtime deserialization with pre-allocated buffers.

use crate::{
    ComparisonOperator, ConditionExpr, LogicalOperator, Predicate, ProtocolError, WebUIFragment,
    WebUIFragmentComponent, WebUIFragmentFor, WebUIFragmentIf, WebUIFragmentRaw,
    WebUIFragmentSignal, WebUIProtocol,
};
use prost::Message;
use std::collections::HashMap;

/// Generated protobuf types (internal).
mod proto {
    include!(concat!(env!("OUT_DIR"), "/webui.rs"));
}

// ── Domain → Proto conversions ──────────────────────────────────────────

impl From<&LogicalOperator> for proto::LogicalOperator {
    fn from(op: &LogicalOperator) -> Self {
        match op {
            LogicalOperator::And => proto::LogicalOperator::And,
            LogicalOperator::Or => proto::LogicalOperator::Or,
        }
    }
}

impl From<&ComparisonOperator> for proto::ComparisonOperator {
    fn from(op: &ComparisonOperator) -> Self {
        match op {
            ComparisonOperator::GreaterThan => proto::ComparisonOperator::GreaterThan,
            ComparisonOperator::LessThan => proto::ComparisonOperator::LessThan,
            ComparisonOperator::Equal => proto::ComparisonOperator::Equal,
            ComparisonOperator::NotEqual => proto::ComparisonOperator::NotEqual,
            ComparisonOperator::GreaterThanOrEqual => proto::ComparisonOperator::GreaterThanOrEqual,
            ComparisonOperator::LessThanOrEqual => proto::ComparisonOperator::LessThanOrEqual,
        }
    }
}

impl From<&ConditionExpr> for proto::ConditionExpr {
    fn from(expr: &ConditionExpr) -> Self {
        let inner = match expr {
            ConditionExpr::Predicate(pred) => {
                proto::condition_expr::Expr::Predicate(proto::Predicate {
                    left: pred.left.clone(),
                    operator: proto::ComparisonOperator::from(&pred.operator) as i32,
                    right: pred.right.clone(),
                })
            }
            ConditionExpr::Not(inner) => {
                proto::condition_expr::Expr::Not(Box::new(proto::NotCondition {
                    condition: Some(Box::new(proto::ConditionExpr::from(inner.as_ref()))),
                }))
            }
            ConditionExpr::Compound { left, op, right } => {
                proto::condition_expr::Expr::Compound(Box::new(proto::CompoundCondition {
                    left: Some(Box::new(proto::ConditionExpr::from(left.as_ref()))),
                    op: proto::LogicalOperator::from(op) as i32,
                    right: Some(Box::new(proto::ConditionExpr::from(right.as_ref()))),
                }))
            }
            ConditionExpr::Identifier { value } => {
                proto::condition_expr::Expr::Identifier(proto::IdentifierCondition {
                    value: value.clone(),
                })
            }
        };
        proto::ConditionExpr { expr: Some(inner) }
    }
}

impl From<&WebUIFragment> for proto::WebUiFragment {
    fn from(fragment: &WebUIFragment) -> Self {
        let inner = match fragment {
            WebUIFragment::Raw(raw) => {
                proto::web_ui_fragment::Fragment::Raw(proto::WebUiFragmentRaw {
                    value: raw.value.clone(),
                })
            }
            WebUIFragment::Component(comp) => {
                proto::web_ui_fragment::Fragment::Component(proto::WebUiFragmentComponent {
                    fragment_id: comp.fragment_id.clone(),
                })
            }
            WebUIFragment::For(for_loop) => {
                proto::web_ui_fragment::Fragment::ForLoop(proto::WebUiFragmentFor {
                    item: for_loop.item.clone(),
                    collection: for_loop.collection.clone(),
                    fragment_id: for_loop.fragment_id.clone(),
                })
            }
            WebUIFragment::Signal(signal) => {
                proto::web_ui_fragment::Fragment::Signal(proto::WebUiFragmentSignal {
                    value: signal.value.clone(),
                    raw: signal.raw,
                })
            }
            WebUIFragment::If(if_cond) => {
                proto::web_ui_fragment::Fragment::IfCond(proto::WebUiFragmentIf {
                    condition: Some(proto::ConditionExpr::from(&if_cond.condition)),
                    fragment_id: if_cond.fragment_id.clone(),
                })
            }
        };
        proto::WebUiFragment {
            fragment: Some(inner),
        }
    }
}

impl From<&WebUIProtocol> for proto::WebUiProtocol {
    fn from(protocol: &WebUIProtocol) -> Self {
        let fragments = protocol
            .fragments
            .iter()
            .map(|(key, frags)| {
                let list = proto::FragmentList {
                    fragments: frags.iter().map(proto::WebUiFragment::from).collect(),
                };
                (key.clone(), list)
            })
            .collect();
        proto::WebUiProtocol { fragments }
    }
}

// ── Proto → Domain conversions ──────────────────────────────────────────

fn logical_op_from_i32(v: i32) -> Result<LogicalOperator, ProtocolError> {
    match proto::LogicalOperator::try_from(v) {
        Ok(proto::LogicalOperator::And) => Ok(LogicalOperator::And),
        Ok(proto::LogicalOperator::Or) => Ok(LogicalOperator::Or),
        _ => Err(ProtocolError::Validation(format!(
            "Invalid logical operator value: {v}"
        ))),
    }
}

fn comparison_op_from_i32(v: i32) -> Result<ComparisonOperator, ProtocolError> {
    match proto::ComparisonOperator::try_from(v) {
        Ok(proto::ComparisonOperator::GreaterThan) => Ok(ComparisonOperator::GreaterThan),
        Ok(proto::ComparisonOperator::LessThan) => Ok(ComparisonOperator::LessThan),
        Ok(proto::ComparisonOperator::Equal) => Ok(ComparisonOperator::Equal),
        Ok(proto::ComparisonOperator::NotEqual) => Ok(ComparisonOperator::NotEqual),
        Ok(proto::ComparisonOperator::GreaterThanOrEqual) => {
            Ok(ComparisonOperator::GreaterThanOrEqual)
        }
        Ok(proto::ComparisonOperator::LessThanOrEqual) => Ok(ComparisonOperator::LessThanOrEqual),
        _ => Err(ProtocolError::Validation(format!(
            "Invalid comparison operator value: {v}"
        ))),
    }
}

fn condition_from_proto(expr: proto::ConditionExpr) -> Result<ConditionExpr, ProtocolError> {
    match expr.expr {
        Some(proto::condition_expr::Expr::Predicate(pred)) => {
            Ok(ConditionExpr::Predicate(Predicate {
                left: pred.left,
                operator: comparison_op_from_i32(pred.operator)?,
                right: pred.right,
            }))
        }
        Some(proto::condition_expr::Expr::Not(not)) => {
            let inner = not
                .condition
                .ok_or_else(|| {
                    ProtocolError::Validation("Not condition missing inner expression".into())
                })
                .and_then(|c| condition_from_proto(*c))?;
            Ok(ConditionExpr::Not(Box::new(inner)))
        }
        Some(proto::condition_expr::Expr::Compound(compound)) => {
            let left = compound
                .left
                .ok_or_else(|| {
                    ProtocolError::Validation("Compound condition missing left expression".into())
                })
                .and_then(|c| condition_from_proto(*c))?;
            let right = compound
                .right
                .ok_or_else(|| {
                    ProtocolError::Validation("Compound condition missing right expression".into())
                })
                .and_then(|c| condition_from_proto(*c))?;
            let op = logical_op_from_i32(compound.op)?;
            Ok(ConditionExpr::Compound {
                left: Box::new(left),
                op,
                right: Box::new(right),
            })
        }
        Some(proto::condition_expr::Expr::Identifier(id)) => {
            Ok(ConditionExpr::Identifier { value: id.value })
        }
        None => Err(ProtocolError::Validation(
            "ConditionExpr has no expression set".into(),
        )),
    }
}

fn fragment_from_proto(f: proto::WebUiFragment) -> Result<WebUIFragment, ProtocolError> {
    match f.fragment {
        Some(proto::web_ui_fragment::Fragment::Raw(raw)) => {
            Ok(WebUIFragment::Raw(WebUIFragmentRaw { value: raw.value }))
        }
        Some(proto::web_ui_fragment::Fragment::Component(comp)) => {
            Ok(WebUIFragment::Component(WebUIFragmentComponent {
                fragment_id: comp.fragment_id,
            }))
        }
        Some(proto::web_ui_fragment::Fragment::ForLoop(fl)) => {
            Ok(WebUIFragment::For(WebUIFragmentFor {
                item: fl.item,
                collection: fl.collection,
                fragment_id: fl.fragment_id,
            }))
        }
        Some(proto::web_ui_fragment::Fragment::Signal(sig)) => {
            Ok(WebUIFragment::Signal(WebUIFragmentSignal {
                value: sig.value,
                raw: sig.raw,
            }))
        }
        Some(proto::web_ui_fragment::Fragment::IfCond(ic)) => {
            let condition = ic
                .condition
                .ok_or_else(|| ProtocolError::Validation("If fragment missing condition".into()))
                .and_then(condition_from_proto)?;
            Ok(WebUIFragment::If(WebUIFragmentIf {
                condition,
                fragment_id: ic.fragment_id,
            }))
        }
        None => Err(ProtocolError::Validation("Fragment has no type set".into())),
    }
}

fn protocol_from_proto(proto: proto::WebUiProtocol) -> Result<WebUIProtocol, ProtocolError> {
    let mut fragments: HashMap<String, Vec<WebUIFragment>> =
        HashMap::with_capacity(proto.fragments.len());

    for (key, list) in proto.fragments {
        let mut frags = Vec::with_capacity(list.fragments.len());
        for f in list.fragments {
            frags.push(fragment_from_proto(f)?);
        }
        fragments.insert(key, frags);
    }

    Ok(WebUIProtocol { fragments })
}

// ── Public API on WebUIProtocol ─────────────────────────────────────────

impl WebUIProtocol {
    /// Serialize protocol to protobuf binary format.
    ///
    /// Uses a pre-allocated buffer sized to the encoded length to avoid
    /// reallocations during serialization.
    pub fn to_protobuf(&self) -> Result<Vec<u8>, ProtocolError> {
        let proto_msg = proto::WebUiProtocol::from(self);
        let len = proto_msg.encoded_len();
        let mut buf = Vec::with_capacity(len);
        proto_msg
            .encode(&mut buf)
            .map_err(|e| ProtocolError::Validation(format!("Protobuf encode error: {e}")))?;
        Ok(buf)
    }

    /// Deserialize protocol from protobuf binary bytes with validation.
    pub fn from_protobuf(bytes: &[u8]) -> crate::Result<Self> {
        let proto_msg = proto::WebUiProtocol::decode(bytes)
            .map_err(|e| ProtocolError::Validation(format!("Protobuf decode error: {e}")))?;
        let protocol = protocol_from_proto(proto_msg)?;
        Self::validate_protocol(protocol)
    }

    /// Read and deserialize a protobuf file with validation.
    pub fn from_protobuf_file<P: AsRef<std::path::Path>>(path: P) -> crate::Result<Self> {
        let bytes = std::fs::read(path)?;
        Self::from_protobuf(&bytes)
    }

    /// Write protocol to a protobuf file.
    pub fn to_protobuf_file<P: AsRef<std::path::Path>>(&self, path: P) -> crate::Result<()> {
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
            vec![
                WebUIFragment::Raw(WebUIFragmentRaw {
                    value: "Hello, WebUI!\n".to_string(),
                }),
                WebUIFragment::For(WebUIFragmentFor {
                    item: "person".to_string(),
                    collection: "people".to_string(),
                    fragment_id: "for-1".to_string(),
                }),
                WebUIFragment::Signal(WebUIFragmentSignal {
                    value: "description".to_string(),
                    raw: true,
                }),
                WebUIFragment::If(WebUIFragmentIf {
                    condition: ConditionExpr::Identifier {
                        value: "contact".to_string(),
                    },
                    fragment_id: "if-1".to_string(),
                }),
            ],
        );
        fragments.insert(
            "for-1".to_string(),
            vec![WebUIFragment::Signal(WebUIFragmentSignal {
                value: "person.name".to_string(),
                raw: false,
            })],
        );
        fragments.insert(
            "if-1".to_string(),
            vec![WebUIFragment::Component(WebUIFragmentComponent {
                fragment_id: "contact-card".to_string(),
            })],
        );
        fragments.insert(
            "contact-card".to_string(),
            vec![
                WebUIFragment::Raw(WebUIFragmentRaw {
                    value: "Hello, ".to_string(),
                }),
                WebUIFragment::Signal(WebUIFragmentSignal {
                    value: "name".to_string(),
                    raw: false,
                }),
            ],
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
            vec![
                WebUIFragment::Raw(WebUIFragmentRaw {
                    value: "text".to_string(),
                }),
                WebUIFragment::Component(WebUIFragmentComponent {
                    fragment_id: "comp".to_string(),
                }),
                WebUIFragment::For(WebUIFragmentFor {
                    item: "x".to_string(),
                    collection: "xs".to_string(),
                    fragment_id: "loop".to_string(),
                }),
                WebUIFragment::Signal(WebUIFragmentSignal {
                    value: "sig".to_string(),
                    raw: true,
                }),
                WebUIFragment::If(WebUIFragmentIf {
                    condition: ConditionExpr::Predicate(Predicate {
                        left: "a".to_string(),
                        operator: ComparisonOperator::GreaterThan,
                        right: "1".to_string(),
                    }),
                    fragment_id: "cond".to_string(),
                }),
            ],
        );
        fragments.insert(
            "comp".to_string(),
            vec![WebUIFragment::Raw(WebUIFragmentRaw {
                value: "c".to_string(),
            })],
        );
        fragments.insert(
            "loop".to_string(),
            vec![WebUIFragment::Raw(WebUIFragmentRaw {
                value: "l".to_string(),
            })],
        );
        fragments.insert(
            "cond".to_string(),
            vec![WebUIFragment::Raw(WebUIFragmentRaw {
                value: "i".to_string(),
            })],
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
                vec![WebUIFragment::If(WebUIFragmentIf {
                    condition: ConditionExpr::Predicate(Predicate {
                        left: "a".to_string(),
                        operator: op.clone(),
                        right: "b".to_string(),
                    }),
                    fragment_id: "then".to_string(),
                })],
            );
            fragments.insert(
                "then".to_string(),
                vec![WebUIFragment::Raw(WebUIFragmentRaw {
                    value: "ok".to_string(),
                })],
            );
            let p = WebUIProtocol { fragments };
            let bytes = p.to_protobuf().unwrap();
            let decoded = WebUIProtocol::from_protobuf(&bytes).unwrap();
            assert_eq!(p, decoded);
        }
    }

    #[test]
    fn test_protobuf_nested_conditions() {
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

        let mut fragments = HashMap::new();
        fragments.insert(
            "main".to_string(),
            vec![WebUIFragment::If(WebUIFragmentIf {
                condition: nested,
                fragment_id: "then".to_string(),
            })],
        );
        fragments.insert(
            "then".to_string(),
            vec![WebUIFragment::Raw(WebUIFragmentRaw {
                value: "ok".to_string(),
            })],
        );
        let p = WebUIProtocol { fragments };
        let bytes = p.to_protobuf().unwrap();
        let decoded = WebUIProtocol::from_protobuf(&bytes).unwrap();
        assert_eq!(p, decoded);
    }

    #[test]
    fn test_protobuf_compound_or_condition() {
        let compound = ConditionExpr::Compound {
            left: Box::new(ConditionExpr::Identifier {
                value: "isAdmin".to_string(),
            }),
            op: LogicalOperator::Or,
            right: Box::new(ConditionExpr::Identifier {
                value: "isEditor".to_string(),
            }),
        };

        let mut fragments = HashMap::new();
        fragments.insert(
            "main".to_string(),
            vec![WebUIFragment::If(WebUIFragmentIf {
                condition: compound,
                fragment_id: "body".to_string(),
            })],
        );
        fragments.insert(
            "body".to_string(),
            vec![WebUIFragment::Raw(WebUIFragmentRaw {
                value: "yes".to_string(),
            })],
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
        // Empty bytes decode to empty protocol (valid protobuf)
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
        // Build invalid protobuf bytes that reference non-existent fragment
        let mut fragments = HashMap::new();
        fragments.insert(
            "main".to_string(),
            vec![WebUIFragment::Component(WebUIFragmentComponent {
                fragment_id: "does-not-exist".to_string(),
            })],
        );

        let proto_msg = proto::WebUiProtocol::from(&WebUIProtocol { fragments });
        let mut buf = Vec::with_capacity(proto_msg.encoded_len());
        proto_msg.encode(&mut buf).unwrap();

        let result = WebUIProtocol::from_protobuf(&buf);
        assert!(result.is_err());
    }

    #[test]
    fn test_protobuf_validation_catches_missing_for_reference() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "main".to_string(),
            vec![WebUIFragment::For(WebUIFragmentFor {
                item: "item".to_string(),
                collection: "items".to_string(),
                fragment_id: "missing-for".to_string(),
            })],
        );

        let proto_msg = proto::WebUiProtocol::from(&WebUIProtocol { fragments });
        let mut buf = Vec::with_capacity(proto_msg.encoded_len());
        proto_msg.encode(&mut buf).unwrap();

        let result = WebUIProtocol::from_protobuf(&buf);
        assert!(result.is_err());
        if let Err(crate::ProtocolError::Validation(msg)) = result {
            assert!(msg.contains("missing-for"));
        }
    }

    #[test]
    fn test_protobuf_validation_catches_missing_if_reference() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "main".to_string(),
            vec![WebUIFragment::If(WebUIFragmentIf {
                condition: ConditionExpr::Identifier {
                    value: "flag".to_string(),
                },
                fragment_id: "missing-if".to_string(),
            })],
        );

        let proto_msg = proto::WebUiProtocol::from(&WebUIProtocol { fragments });
        let mut buf = Vec::with_capacity(proto_msg.encoded_len());
        proto_msg.encode(&mut buf).unwrap();

        let result = WebUIProtocol::from_protobuf(&buf);
        assert!(result.is_err());
        if let Err(crate::ProtocolError::Validation(msg)) = result {
            assert!(msg.contains("missing-if"));
        }
    }

    #[test]
    fn test_protobuf_signal_default_raw_false() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "main".to_string(),
            vec![WebUIFragment::Signal(WebUIFragmentSignal {
                value: "name".to_string(),
                raw: false,
            })],
        );
        let p = WebUIProtocol { fragments };
        let bytes = p.to_protobuf().unwrap();
        let decoded = WebUIProtocol::from_protobuf(&bytes).unwrap();
        let sig = match &decoded.fragments["main"][0] {
            WebUIFragment::Signal(s) => s,
            _ => panic!("expected signal"),
        };
        assert!(!sig.raw);
    }

    #[test]
    fn test_protobuf_pre_allocated_buffer() {
        let protocol = sample_protocol();
        let bytes = protocol.to_protobuf().unwrap();
        let proto_msg = proto::WebUiProtocol::from(&protocol);
        // Buffer should be exactly the encoded length
        assert_eq!(bytes.len(), proto_msg.encoded_len());
    }
}
