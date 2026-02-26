use super::*;

#[test]
fn test_handle_raw() {
    // Create a simple protocol
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("Hello, WebUI!")],
        },
    );

    let protocol = WebUIProtocol { fragments };
    let state = test_json!({});

    // Create a test writer
    let mut writer = TestWriter::new();

    // Handle the protocol
    assert!(
        handle(&protocol, &state, &mut writer).is_ok(),
        "Failed to handle raw protocol"
    );

    // Check the output
    assert_eq!(writer.get_content(), "Hello, WebUI!");
    assert!(writer.is_ended());
}

#[test]
fn test_handle_signal() {
    // Create a protocol with a signal
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("Hello, "),
                WebUIFragment::signal("name", false),
                WebUIFragment::raw("!"),
            ],
        },
    );

    let protocol = WebUIProtocol { fragments };
    let state = test_json!({"name": "WebUI"});

    // Create a test writer
    let mut writer = TestWriter::new();

    // Handle the protocol
    assert!(
        handle(&protocol, &state, &mut writer).is_ok(),
        "Failed to handle signal protocol"
    );

    // Check the output
    assert_eq!(writer.get_content(), "Hello, WebUI!");
    assert!(writer.is_ended());
}

#[test]
fn test_handle_for_loop() {
    // Create a protocol with a for loop
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("People: "),
                WebUIFragment::for_loop("person", "people", "person-item"),
            ],
        },
    );

    fragments.insert(
        "person-item".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::signal("person.name", false),
                WebUIFragment::raw(", "),
            ],
        },
    );

    let protocol = WebUIProtocol { fragments };
    let state = test_json!({
        "people": [
            {"name": "Alice"},
            {"name": "Bob"},
            {"name": "Charlie"}
        ]
    });

    // Create a test writer
    let mut writer = TestWriter::new();

    // Handle the protocol
    assert!(
        handle(&protocol, &state, &mut writer).is_ok(),
        "Failed to handle for loop protocol"
    );

    // Check the output
    assert_eq!(writer.get_content(), "People: Alice, Bob, Charlie, ");
    assert!(writer.is_ended());
}

#[test]
fn test_handle_if_condition() {
    // Create a protocol with an if condition
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("Status: "),
                WebUIFragment::if_cond(
                    webui_protocol::ConditionExpr::identifier("isActive"),
                    "active-content",
                ),
                WebUIFragment::raw("End"),
            ],
        },
    );

    fragments.insert(
        "active-content".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("Active")],
        },
    );

    let protocol = WebUIProtocol { fragments };

    // Test with isActive = true
    let state_true = test_json!({"isActive": true});
    let mut writer_true = TestWriter::new();
    assert!(
        handle(&protocol, &state_true, &mut writer_true).is_ok(),
        "Failed to handle if condition (true case)"
    );
    assert_eq!(writer_true.get_content(), "Status: ActiveEnd");
    assert!(writer_true.is_ended());

    // Test with isActive = false
    let state_false = test_json!({"isActive": false});
    let mut writer_false = TestWriter::new();
    assert!(
        handle(&protocol, &state_false, &mut writer_false).is_ok(),
        "Failed to handle if condition (false case)"
    );
    assert_eq!(writer_false.get_content(), "Status: End");
    assert!(writer_false.is_ended());
}

#[test]
fn test_handle_component() {
    // Create a protocol with a component
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("Component: "),
                WebUIFragment::component("my-component"),
            ],
        },
    );

    fragments.insert(
        "my-component".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("<div>Component Content</div>")],
        },
    );

    let protocol = WebUIProtocol { fragments };
    let state = test_json!({});

    // Create a test writer
    let mut writer = TestWriter::new();

    // Handle the protocol
    assert!(
        handle(&protocol, &state, &mut writer).is_ok(),
        "Failed to handle component protocol"
    );

    // Check the output
    assert_eq!(
        writer.get_content(),
        "Component: <div>Component Content</div>"
    );
    assert!(writer.is_ended());
}

#[test]
fn test_missing_fragment() {
    // Create a protocol with a missing fragment reference
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::component("missing-component")],
        },
    );

    let protocol = WebUIProtocol { fragments };
    let state = test_json!({});

    // Create a test writer
    let mut writer = TestWriter::new();

    // Handle the protocol
    let result = handle(&protocol, &state, &mut writer);

    // Expect an error
    assert!(result.is_err());
    if let Err(HandlerError::MissingFragment(fragment_id)) = result {
        assert_eq!(fragment_id, "missing-component");
    } else {
        panic!("Expected MissingFragment error");
    }
}

#[test]
fn test_missing_signal_renders_empty() {
    // A signal referencing a field absent from state should render as empty
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("Hello, "),
                WebUIFragment::signal("missing_field", false),
                WebUIFragment::raw("!"),
            ],
        },
    );

    let protocol = WebUIProtocol { fragments };
    let state = test_json!({});

    let mut writer = TestWriter::new();

    assert!(
        handle(&protocol, &state, &mut writer).is_ok(),
        "Missing signal should not produce an error"
    );

    assert_eq!(writer.get_content(), "Hello, !");
    assert!(writer.is_ended());
}

#[test]
fn test_raw_signal_not_escaped() {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::signal("html", false),
                WebUIFragment::signal("html", true),
            ],
        },
    );
    let protocol = WebUIProtocol { fragments };
    let state = test_json!({"html": "<strong>hi</strong>"});
    let mut writer = TestWriter::new();
    handle(&protocol, &state, &mut writer).unwrap();
    assert_eq!(
        writer.get_content(),
        "&lt;strong&gt;hi&lt;&#x2F;strong&gt;<strong>hi</strong>"
    );
}
