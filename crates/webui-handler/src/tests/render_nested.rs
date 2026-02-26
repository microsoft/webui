use super::*;

// ── Nested for loop tests ─────────────────────────────────────────

#[test]
fn test_nested_for_loop() {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<div>"),
                WebUIFragment::for_loop("outerItem", "outerItems", "outer"),
                WebUIFragment::raw("</div>"),
            ],
        },
    );
    fragments.insert(
        "outer".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<div>"),
                WebUIFragment::for_loop("innerItem", "outerItem.innerItems", "inner"),
                WebUIFragment::raw("</div>"),
            ],
        },
    );
    fragments.insert(
        "inner".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("<span>Inner</span>")],
        },
    );
    let protocol = WebUIProtocol { fragments };
    let state = test_json!({
        "outerItems": [
            {"innerItems": [{"name": "A"}, {"name": "B"}]},
            {"innerItems": [{"name": "C"}]}
        ]
    });
    let mut writer = TestWriter::new();
    handle(&protocol, &state, &mut writer).unwrap();
    assert_eq!(
        writer.get_content(),
        "<div><div><span>Inner</span><span>Inner</span></div><div><span>Inner</span></div></div>"
    );
}

#[test]
fn test_nested_for_with_signals() {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::for_loop("item", "items", "item-tpl")],
        },
    );
    fragments.insert(
        "item-tpl".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<p>"),
                WebUIFragment::signal("item.name", false),
                WebUIFragment::raw("</p>"),
            ],
        },
    );
    let protocol = WebUIProtocol { fragments };
    let state = test_json!({"items": [{"name": "Alice"}, {"name": "Bob"}]});
    let mut writer = TestWriter::new();
    handle(&protocol, &state, &mut writer).unwrap();
    assert_eq!(writer.get_content(), "<p>Alice</p><p>Bob</p>");
}

#[test]
fn test_nested_for_with_global_state() {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::for_loop("item", "items", "item-tpl")],
        },
    );
    fragments.insert(
        "item-tpl".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::signal("item.name", false),
                WebUIFragment::raw(" - "),
                WebUIFragment::signal("globalTitle", false),
            ],
        },
    );
    let protocol = WebUIProtocol { fragments };
    let state = test_json!({"items": [{"name": "A"}, {"name": "B"}], "globalTitle": "Title"});
    let mut writer = TestWriter::new();
    handle(&protocol, &state, &mut writer).unwrap();
    assert_eq!(writer.get_content(), "A - TitleB - Title");
}

// ── For + If state scoping tests ──────────────────────────────────

#[test]
fn test_if_in_for_uses_local_state() {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::for_loop("item", "items", "item-tpl")],
        },
    );
    fragments.insert(
        "item-tpl".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::if_cond(
                ConditionExpr::identifier("item.visible"),
                "visible-tpl",
            )],
        },
    );
    fragments.insert(
        "visible-tpl".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::signal("item.name", false)],
        },
    );
    let protocol = WebUIProtocol { fragments };
    let state = test_json!({"items": [{"name": "Show", "visible": true}, {"name": "Hide", "visible": false}]});
    let mut writer = TestWriter::new();
    handle(&protocol, &state, &mut writer).unwrap();
    assert_eq!(writer.get_content(), "Show");
}

#[test]
fn test_for_if_local_overrides_global() {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::for_loop("item", "items", "item-tpl")],
        },
    );
    fragments.insert(
        "item-tpl".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::if_cond(
                ConditionExpr::identifier("item.flag"),
                "show-tpl",
            )],
        },
    );
    fragments.insert(
        "show-tpl".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("yes")],
        },
    );
    let protocol = WebUIProtocol { fragments };
    // Global flag is true, but local item.flag is false for second item
    let state = test_json!({"flag": true, "items": [{"flag": true}, {"flag": false}]});
    let mut writer = TestWriter::new();
    handle(&protocol, &state, &mut writer).unwrap();
    assert_eq!(writer.get_content(), "yes");
}

// ── Component attribute state tests ───────────────────────────────

#[test]
fn test_component_attr_state_simple() {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<my-comp"),
                WebUIFragment {
                    fragment: Some(web_ui_fragment::Fragment::Attribute(
                        WebUIFragmentAttribute {
                            name: "title".into(),
                            value: "Attribute Title".into(),
                            attr_start: true,
                            raw_value: true,
                            ..Default::default()
                        },
                    )),
                },
                WebUIFragment::raw(">"),
                WebUIFragment::component("my-comp"),
                WebUIFragment::raw("</my-comp>"),
            ],
        },
    );
    fragments.insert(
        "my-comp".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<span>"),
                WebUIFragment::signal("title", false),
                WebUIFragment::raw("</span>"),
            ],
        },
    );
    let protocol = WebUIProtocol { fragments };
    let state = test_json!({"title": "Global Title"});
    let mut writer = TestWriter::new();
    handle(&protocol, &state, &mut writer).unwrap();
    assert_eq!(
        writer.get_content(),
        "<my-comp title=\"Attribute Title\"><span>Attribute Title</span></my-comp>"
    );
}

#[test]
fn test_component_attr_state_template() {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<my-comp"),
                WebUIFragment {
                    fragment: Some(web_ui_fragment::Fragment::Attribute(
                        WebUIFragmentAttribute {
                            name: "title".into(),
                            template: "title-attr".into(),
                            attr_start: true,
                            ..Default::default()
                        },
                    )),
                },
                WebUIFragment::raw(">"),
                WebUIFragment::component("my-comp"),
                WebUIFragment::raw("</my-comp>"),
            ],
        },
    );
    fragments.insert(
        "title-attr".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("hello "),
                WebUIFragment::signal("item", false),
            ],
        },
    );
    fragments.insert(
        "my-comp".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<span>"),
                WebUIFragment::signal("title", false),
                WebUIFragment::raw("</span>"),
            ],
        },
    );
    let protocol = WebUIProtocol { fragments };
    let state = test_json!({"item": "<world>"});
    let mut writer = TestWriter::new();
    handle(&protocol, &state, &mut writer).unwrap();
    assert_eq!(
        writer.get_content(),
        "<my-comp title=\"hello &lt;world&gt;\"><span>hello &lt;world&gt;</span></my-comp>"
    );
}

#[test]
fn test_component_attr_camel_case() {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<my-comp"),
                WebUIFragment {
                    fragment: Some(web_ui_fragment::Fragment::Attribute(
                        WebUIFragmentAttribute {
                            name: "data-title".into(),
                            template: "dt-attr".into(),
                            attr_start: true,
                            ..Default::default()
                        },
                    )),
                },
                WebUIFragment::raw(">"),
                WebUIFragment::component("my-comp"),
                WebUIFragment::raw("</my-comp>"),
            ],
        },
    );
    fragments.insert(
        "dt-attr".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("prefix "),
                WebUIFragment::signal("item", false),
            ],
        },
    );
    fragments.insert(
        "my-comp".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<span>"),
                WebUIFragment::signal("dataTitle", false),
                WebUIFragment::raw("</span>"),
            ],
        },
    );
    let protocol = WebUIProtocol { fragments };
    let state = test_json!({"item": "a&b"});
    let mut writer = TestWriter::new();
    handle(&protocol, &state, &mut writer).unwrap();
    assert_eq!(
        writer.get_content(),
        "<my-comp data-title=\"prefix a&amp;b\"><span>prefix a&amp;b</span></my-comp>"
    );
}

#[test]
fn test_component_complex_attr() {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<my-comp"),
                WebUIFragment {
                    fragment: Some(web_ui_fragment::Fragment::Attribute(
                        WebUIFragmentAttribute {
                            name: ":item".into(),
                            value: "complexItem".into(),
                            attr_start: true,
                            complex: true,
                            ..Default::default()
                        },
                    )),
                },
                WebUIFragment::raw(">"),
                WebUIFragment::component("my-comp"),
                WebUIFragment::raw("</my-comp>"),
            ],
        },
    );
    fragments.insert(
        "my-comp".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<span>"),
                WebUIFragment::signal("item.foo", false),
                WebUIFragment::raw("</span><p>"),
                WebUIFragment::signal("item.bar", false),
                WebUIFragment::raw("</p>"),
            ],
        },
    );
    let protocol = WebUIProtocol { fragments };
    let state = test_json!({"complexItem": {"foo": 1, "bar": "true"}});
    let mut writer = TestWriter::new();
    handle(&protocol, &state, &mut writer).unwrap();
    assert_eq!(
        writer.get_content(),
        "<my-comp><span>1</span><p>true</p></my-comp>"
    );
}

#[test]
fn test_component_no_parent_pollution() {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<parent"),
                WebUIFragment {
                    fragment: Some(web_ui_fragment::Fragment::Attribute(
                        WebUIFragmentAttribute {
                            name: "var".into(),
                            value: "var".into(),
                            attr_start: true,
                            ..Default::default()
                        },
                    )),
                },
                WebUIFragment::raw(">"),
                WebUIFragment::component("parent"),
                WebUIFragment::raw("</parent>"),
            ],
        },
    );
    fragments.insert(
        "parent".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("Before: "),
                WebUIFragment::signal("var", false),
                WebUIFragment::raw("<child foo"),
                WebUIFragment {
                    fragment: Some(web_ui_fragment::Fragment::Attribute(
                        WebUIFragmentAttribute {
                            name: "var".into(),
                            value: "replaced".into(),
                            raw_value: true,
                            ..Default::default()
                        },
                    )),
                },
                WebUIFragment::raw(">"),
                WebUIFragment::component("child"),
                WebUIFragment::raw("Label</child>After: "),
                WebUIFragment::signal("var", false),
            ],
        },
    );
    fragments.insert("child".to_string(), FragmentList { fragments: vec![] });
    let protocol = WebUIProtocol { fragments };
    let state = test_json!({"var": "original"});
    let mut writer = TestWriter::new();
    handle(&protocol, &state, &mut writer).unwrap();
    assert_eq!(
        writer.get_content(),
        "<parent var=\"original\">Before: original<child foo var=\"replaced\">Label</child>After: original</parent>"
    );
}

#[test]
fn test_component_boolean_attr_state() {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<my-comp"),
                WebUIFragment {
                    fragment: Some(web_ui_fragment::Fragment::Attribute(
                        WebUIFragmentAttribute {
                            name: "disabled".into(),
                            attr_start: true,
                            condition_tree: Some(ConditionExpr::identifier("isDisabled")),
                            ..Default::default()
                        },
                    )),
                },
                WebUIFragment::raw(">"),
                WebUIFragment::component("my-comp"),
                WebUIFragment::raw("</my-comp>"),
            ],
        },
    );
    fragments.insert(
        "my-comp".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::if_cond(
                ConditionExpr::identifier("disabled"),
                "show",
            )],
        },
    );
    fragments.insert(
        "show".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("disabled!")],
        },
    );
    let protocol = WebUIProtocol { fragments };
    let state = test_json!({"isDisabled": true});
    let mut writer = TestWriter::new();
    handle(&protocol, &state, &mut writer).unwrap();
    assert_eq!(
        writer.get_content(),
        "<my-comp disabled>disabled!</my-comp>"
    );
}
