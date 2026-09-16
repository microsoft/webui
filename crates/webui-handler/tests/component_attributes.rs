// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use webui_handler::{RenderOptions, ResponseWriter, WebUIHandler};
use webui_parser::{ComponentRegistration, HtmlParser};
use webui_protocol::WebUIProtocol;
use webui_test_utils::test_json;

#[derive(Default)]
struct Writer(String);

impl ResponseWriter for Writer {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.0.push_str(content);
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

fn render(entry: &str, state: &serde_json::Value) -> String {
    render_with_attributes(entry, state, &[])
}

fn render_with_attributes(
    entry: &str,
    state: &serde_json::Value,
    attributes: &[(&str, &str, bool)],
) -> String {
    let mut parser = HtmlParser::new();
    parser
        .component_registry_mut()
        .register_component(ComponentRegistration::new(
            "test-dialog",
            r#"<dialog ?open="{{open}}" aria-label="{{ariaLabel}}" aria-describedby="{{ariaDescribedBy}}">{{label}}</dialog>"#,
            None,
            true,
        ))
        .unwrap();
    parser
        .component_registry_mut()
        .register_component(ComponentRegistration::new(
            "test-parent",
            r#"<test-dialog ?open="{{open}}" aria-label="{{ariaLabel}}"></test-dialog><span>{{ariaLabel}}</span>"#,
            None,
            true,
        ))
        .unwrap();
    parser.parse("index.html", entry).unwrap();
    let mut protocol = WebUIProtocol::new(parser.into_fragment_records());
    for fragment in &mut protocol.fragments.get_mut("index.html").unwrap().fragments {
        if let Some(webui_protocol::web_ui_fragment::Fragment::Attribute(attr)) =
            &mut fragment.fragment
        {
            for &(name, property, boolean) in attributes {
                if attr.name == name {
                    attr.property = property.into();
                    attr.boolean = boolean;
                    attr.attr_skip = false;
                }
            }
        }
    }
    protocol.components.insert(
        "test-dialog".into(),
        webui_protocol::ComponentData {
            uses_shadow_dom: true,
            ..Default::default()
        },
    );
    protocol.components.insert(
        "test-parent".into(),
        webui_protocol::ComponentData {
            uses_shadow_dom: true,
            ..Default::default()
        },
    );
    protocol.populate_style_closures(&["index.html"]);
    let mut writer = Writer::default();
    WebUIHandler::new()
        .render(
            &webui_handler::Protocol::new(protocol),
            state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
    writer.0
}

#[test]
fn literal_boolean_presence_and_aria_attributes_reach_component_scope() {
    for attribute in ["open", r#"open="""#, r#"open="open""#, r#"open="false""#] {
        let html = render(
            &format!(
                r#"<test-dialog {attribute} aria-label="Canvas information" aria-describedby="details"></test-dialog>"#
            ),
            &test_json!({"open": false, "ariaLabel": "Global name"}),
        );
        assert!(
            html.contains(
                r#"<dialog open aria-label="Canvas information" aria-describedby="details">"#
            ),
            "{attribute}: {html}"
        );
    }
}

#[test]
fn empty_literals_override_global_state_and_remain_present() {
    let html = render(
        r#"<test-dialog aria-label="" label=""></test-dialog>"#,
        &test_json!({"ariaLabel": "Global name", "label": "Global label"}),
    );
    assert!(html.contains(r#"aria-label="" label="">"#), "{html}");
    assert!(
        html.contains(r#"<dialog aria-label="" aria-describedby=""></dialog>"#),
        "{html}"
    );
}

#[test]
fn sibling_instances_keep_independent_attributes_and_absence_defaults() {
    let html = render(
        r#"<test-dialog open aria-label="First"></test-dialog><test-dialog></test-dialog>"#,
        &test_json!({"open": false, "ariaLabel": "Default"}),
    );
    assert!(
        html.contains(r#"<dialog open aria-label="First""#),
        "{html}"
    );
    assert!(html.contains(r#"<dialog aria-label="Default""#), "{html}");
    assert_eq!(html.matches("<dialog open").count(), 1, "{html}");
}

#[test]
fn dynamic_boolean_false_overrides_global_true_without_leaking_native_attrs() {
    let html = render(
        r#"<input aria-label="{{nativeLabel}}"><test-dialog ?open="{{enabled}}" aria-label="{{name}}"></test-dialog>"#,
        &test_json!({"open": true, "enabled": false, "name": "Child", "nativeLabel": "Native"}),
    );
    assert!(html.contains(r#"<dialog aria-label="Child""#), "{html}");
    assert!(!html.contains("<dialog open"), "{html}");
}

#[test]
fn component_attribute_entities_are_decoded_once_for_child_state() {
    let html = render(
        r#"<test-dialog aria-label='Canvas &amp; &quot;details&quot;'></test-dialog>"#,
        &test_json!({}),
    );
    assert_eq!(
        html.matches(r#"aria-label="Canvas &amp; &quot;details&quot;""#)
            .count(),
        2,
        "{html}",
    );
    assert!(!html.contains("&amp;amp;"), "{html}");
}

#[test]
fn ordinary_boolean_attribute_bindings_follow_presence_not_string_truthiness() {
    let html = render(
        r#"<test-dialog open="{{enabled}}" aria-label="Bound"></test-dialog>"#,
        &test_json!({"enabled": false}),
    );
    assert!(html.contains(r#"<test-dialog open="false""#), "{html}");
    assert!(
        html.contains(r#"<dialog open aria-label="Bound""#),
        "{html}"
    );
}

#[test]
fn parent_bindings_and_loop_instances_resolve_before_entering_child_scope() {
    let html = render(
        r#"<for each="item in items"><test-parent ?open="{{item.open}}" aria-label="{{item.name}}"></test-parent></for><test-dialog></test-dialog>"#,
        &test_json!({
            "open": true,
            "ariaLabel": "Global",
            "items": [{"open": false, "name": "Closed"}, {"open": true, "name": "Open"}]
        }),
    );
    assert!(html.contains(r#"<dialog aria-label="Closed""#), "{html}");
    assert!(html.contains(r#"<dialog open aria-label="Open""#), "{html}");
    assert!(html.contains("<span>Closed</span>"), "{html}");
    assert!(html.contains("<span>Open</span>"), "{html}");
    assert!(
        html.contains(r#"<dialog open aria-label="Global""#),
        "{html}"
    );
}

#[test]
fn projected_modes_distinguish_string_boolean_and_direct_property_inputs() {
    let state = test_json!({"enabled": false, "open": true});
    let string = render_with_attributes(
        r#"<test-dialog open="{{enabled}}"></test-dialog>"#,
        &state,
        &[("open", "open", false)],
    );
    assert!(string.contains("<dialog open"), "{string}");
    let empty = render_with_attributes(
        r#"<test-dialog open=""></test-dialog>"#,
        &state,
        &[("open", "open", false)],
    );
    assert!(!empty.contains("<dialog open"), "{empty}");
    let boolean = render_with_attributes(
        r#"<test-dialog data-expanded="" dialog-name="Exact alias"></test-dialog>"#,
        &test_json!({"open": false}),
        &[
            ("data-expanded", "open", true),
            ("dialog-name", "ariaLabel", false),
        ],
    );
    assert!(
        boolean.contains(r#"<dialog open aria-label="Exact alias""#),
        "{boolean}"
    );
    let direct = render_with_attributes(
        r#"<test-dialog :open="{{enabled}}"></test-dialog>"#,
        &state,
        &[(":open", "open", true)],
    );
    assert!(!direct.contains("<dialog open"), "{direct}");
}

#[test]
fn legacy_static_protocol_literals_keep_their_verbatim_output() {
    use webui_protocol::{
        web_ui_fragment::Fragment, FragmentList, WebUIFragment, WebUIFragmentAttribute,
    };
    let protocol = WebUIProtocol::new(std::collections::HashMap::from([(
        "index.html".into(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<p"),
                WebUIFragment {
                    fragment: Some(Fragment::Attribute(WebUIFragmentAttribute {
                        name: "title".into(),
                        value: "A &amp; B".into(),
                        raw_value: true,
                        ..Default::default()
                    })),
                },
                WebUIFragment::raw("></p>"),
            ],
            contains_boundary: false,
        },
    )]));
    let mut writer = Writer::default();
    WebUIHandler::new()
        .render(
            &webui_handler::Protocol::new(protocol),
            &test_json!({}),
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
    assert_eq!(writer.0, r#"<p title="A &amp; B"></p>"#);
}
