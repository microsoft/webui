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
