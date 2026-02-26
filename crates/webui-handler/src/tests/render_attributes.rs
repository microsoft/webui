use super::*;

// ── Boolean attribute rendering tests ─────────────────────────────

#[test]
fn test_boolean_attr_true() {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<button"),
                WebUIFragment::attribute_boolean(
                    "disabled",
                    ConditionExpr::identifier("isDisabled"),
                ),
                WebUIFragment::raw(">Click</button>"),
            ],
        },
    );
    let protocol = WebUIProtocol { fragments };
    let state = test_json!({"isDisabled": true});
    let mut writer = TestWriter::new();
    handle(&protocol, &state, &mut writer).unwrap();
    assert_eq!(writer.get_content(), "<button disabled>Click</button>");
}

#[test]
fn test_boolean_attr_false() {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<button"),
                WebUIFragment::attribute_boolean(
                    "disabled",
                    ConditionExpr::identifier("isDisabled"),
                ),
                WebUIFragment::raw(">Click</button>"),
            ],
        },
    );
    let protocol = WebUIProtocol { fragments };
    let state = test_json!({"isDisabled": false});
    let mut writer = TestWriter::new();
    handle(&protocol, &state, &mut writer).unwrap();
    assert_eq!(writer.get_content(), "<button>Click</button>");
}

#[test]
fn test_boolean_attr_missing() {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<input type=\"checkbox\""),
                WebUIFragment::attribute_boolean("checked", ConditionExpr::identifier("checked")),
                WebUIFragment::raw(">"),
            ],
        },
    );
    let protocol = WebUIProtocol { fragments };
    let state = test_json!({});
    let mut writer = TestWriter::new();
    handle(&protocol, &state, &mut writer).unwrap();
    assert_eq!(writer.get_content(), "<input type=\"checkbox\">");
}

#[test]
fn test_boolean_attr_multiple() {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<input type=\"checkbox\""),
                WebUIFragment::attribute_boolean("checked", ConditionExpr::identifier("checked")),
                WebUIFragment::attribute_boolean("disabled", ConditionExpr::identifier("disabled")),
                WebUIFragment::raw(">"),
            ],
        },
    );
    let protocol = WebUIProtocol { fragments };
    let state = test_json!({"checked": true, "disabled": false});
    let mut writer = TestWriter::new();
    handle(&protocol, &state, &mut writer).unwrap();
    assert_eq!(writer.get_content(), "<input type=\"checkbox\" checked>");
}

// ── Simple attribute rendering tests ──────────────────────────────

#[test]
fn test_attribute_with_value() {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<input"),
                WebUIFragment::attribute("value", "inputValue"),
                WebUIFragment::raw(">"),
            ],
        },
    );
    let protocol = WebUIProtocol { fragments };
    let state = test_json!({"inputValue": "Hello"});
    let mut writer = TestWriter::new();
    handle(&protocol, &state, &mut writer).unwrap();
    assert_eq!(writer.get_content(), "<input value=\"Hello\">");
}

#[test]
fn test_attribute_with_falsy_numeric() {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<div name=\"test\""),
                WebUIFragment::attribute("handle", "number"),
                WebUIFragment::raw("></div>"),
            ],
        },
    );
    let protocol = WebUIProtocol { fragments };
    let state = test_json!({"number": 0});
    let mut writer = TestWriter::new();
    handle(&protocol, &state, &mut writer).unwrap();
    assert_eq!(
        writer.get_content(),
        "<div name=\"test\" handle=\"0\"></div>"
    );
}

// ── Template attribute rendering tests ────────────────────────────

#[test]
fn test_mixed_attribute_template() {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<input"),
                WebUIFragment::attribute_template("value", "attr-1"),
                WebUIFragment::raw(">"),
            ],
        },
    );
    fragments.insert(
        "attr-1".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("hello "),
                WebUIFragment::signal("item", false),
            ],
        },
    );
    let protocol = WebUIProtocol { fragments };
    let state = test_json!({"item": "world"});
    let mut writer = TestWriter::new();
    handle(&protocol, &state, &mut writer).unwrap();
    assert_eq!(writer.get_content(), "<input value=\"hello world\">");
}
