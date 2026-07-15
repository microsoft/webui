// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use super::*;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use webui::{BuildOptions, Plugin};

struct ExampleCase {
    name: &'static str,
    plugin: Plugin,
    uses_theme: bool,
}

const EXAMPLES: &[ExampleCase] = &[
    ExampleCase {
        name: "calculator",
        plugin: Plugin::WebUI,
        uses_theme: false,
    },
    ExampleCase {
        name: "commerce",
        plugin: Plugin::WebUI,
        uses_theme: false,
    },
    ExampleCase {
        name: "component-assets",
        plugin: Plugin::WebUI,
        uses_theme: true,
    },
    ExampleCase {
        name: "contact-book-manager",
        plugin: Plugin::WebUI,
        uses_theme: true,
    },
    ExampleCase {
        name: "hello-world",
        plugin: Plugin::WebUI,
        uses_theme: true,
    },
    ExampleCase {
        name: "todo-fast",
        plugin: Plugin::FastV3,
        uses_theme: true,
    },
    ExampleCase {
        name: "todo-webui",
        plugin: Plugin::WebUI,
        uses_theme: false,
    },
];

#[test]
fn snapshots_component_scope_schema() {
    let schema = build_fixture_schema("component-scope", "ComponentScopeState");
    insta::assert_json_snapshot!(SchemaDocument::new(&schema));
}

#[test]
fn snapshots_nested_collection_schema() {
    let schema = build_fixture_schema("nested-collections", "NestedCollectionsState");
    insta::assert_json_snapshot!(SchemaDocument::new(&schema));
}

#[test]
fn snapshots_routes_example_schema_bundle() {
    let app_dir = workspace_root().join("examples/app/routes/src");
    let schema = build_schema(&app_dir, Plugin::WebUI, None, "RoutesState");
    insta::assert_json_snapshot!(SchemaDocument::new(&schema));
}

#[test]
fn snapshots_routes_declared_in_component_schema_bundle() {
    let schema = build_fixture_schema("routed-shell", "RoutedShellState");
    insta::assert_json_snapshot!(SchemaDocument::new(&schema));
}

#[test]
fn route_definition_keys_are_injective_for_common_collisions() {
    assert_ne!(route_definition_key("/"), route_definition_key("/root"));
    assert_ne!(
        route_definition_key("/:id"),
        route_definition_key("/param_id")
    );
    assert_ne!(route_definition_key("/A"), route_definition_key("/a"));
    assert_ne!(route_definition_key("/a.b"), route_definition_key("/a/b"));
    assert_eq!(
        route_definition_key("/sections/:sectionId"),
        "route.sections.:sectionId"
    );
}

#[test]
fn encoded_route_references_resolve_to_definition_keys() {
    let paths = ["/docs/v1.0", "/café", "/a b", "/100%"];
    let routes = paths
        .iter()
        .enumerate()
        .map(|(index, path)| WebUIFragmentRoute {
            path: (*path).to_string(),
            fragment_id: format!("page-{index}"),
            exact: true,
            ..WebUIFragmentRoute::default()
        })
        .collect();
    let protocol = protocol_with_entry_routes(routes);
    let schema = generate_schema(&protocol, "index.html", "EncodedRoutesState").unwrap();
    let definitions = schema["$defs"].as_object().unwrap();

    for path in paths {
        let schema_ref = schema["x-webui-routes"][path].as_str().unwrap();
        let key = decode_definition_ref(schema_ref);
        assert!(
            definitions.contains_key(&key),
            "missing definition for {path}"
        );
    }
}

#[test]
fn optional_child_replaces_parent_only_route_chain() {
    let route = WebUIFragmentRoute {
        path: "/".to_string(),
        fragment_id: "shell".to_string(),
        children: vec![WebUIFragmentRoute {
            path: ":tab?".to_string(),
            fragment_id: "tab-page".to_string(),
            exact: true,
            ..WebUIFragmentRoute::default()
        }],
        ..WebUIFragmentRoute::default()
    };
    let protocol = protocol_with_entry_routes(vec![route]);
    let routes = collect_route_chains(&protocol, "index.html").unwrap();

    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].path, "/:tab?");
    assert_eq!(routes[0].components, ["shell", "tab-page"]);
}

#[test]
fn zero_segment_child_detection_matches_route_syntax() {
    assert!(child_matches_parent_path("/parent", ""));
    assert!(child_matches_parent_path("/parent", "./"));
    assert!(child_matches_parent_path("/parent", ":id?"));
    assert!(child_matches_parent_path("/parent", "*rest"));
    assert!(!child_matches_parent_path("/parent", "./child"));
}

#[test]
fn routes_inside_control_flow_fragments_are_discovered() {
    let if_route = WebUIFragmentRoute {
        path: "/conditional".to_string(),
        fragment_id: "conditional-page".to_string(),
        exact: true,
        ..WebUIFragmentRoute::default()
    };
    let loop_route = WebUIFragmentRoute {
        path: "/repeated".to_string(),
        fragment_id: "repeated-page".to_string(),
        exact: true,
        ..WebUIFragmentRoute::default()
    };
    let records = std::collections::HashMap::from([
        (
            "index.html".to_string(),
            webui_protocol::FragmentList {
                fragments: vec![
                    webui_protocol::WebUIFragment::if_cond(
                        webui_protocol::ConditionExpr::identifier("showRoutes"),
                        "if-routes",
                    ),
                    webui_protocol::WebUIFragment::for_loop("item", "routeItems", "for-routes"),
                ],
            },
        ),
        (
            "if-routes".to_string(),
            webui_protocol::FragmentList {
                fragments: vec![webui_protocol::WebUIFragment::route_from(if_route)],
            },
        ),
        (
            "for-routes".to_string(),
            webui_protocol::FragmentList {
                fragments: vec![webui_protocol::WebUIFragment::route_from(loop_route)],
            },
        ),
        (
            "conditional-page".to_string(),
            webui_protocol::FragmentList {
                fragments: Vec::new(),
            },
        ),
        (
            "repeated-page".to_string(),
            webui_protocol::FragmentList {
                fragments: Vec::new(),
            },
        ),
    ]);
    let protocol = WebUIProtocol::new(records);
    let routes = collect_route_chains(&protocol, "index.html").unwrap();
    let paths: Vec<&str> = routes.iter().map(|route| route.path.as_str()).collect();

    assert_eq!(paths, ["/conditional", "/repeated"]);
}

#[test]
fn literal_looking_binding_names_remain_state_paths() {
    let protocol = WebUIProtocol::new(std::collections::HashMap::from([
        (
            "index.html".to_string(),
            webui_protocol::FragmentList {
                fragments: vec![
                    webui_protocol::WebUIFragment::signal("true", false),
                    webui_protocol::WebUIFragment::signal("42", false),
                    webui_protocol::WebUIFragment::if_cond(
                        webui_protocol::ConditionExpr::predicate(
                            "false",
                            webui_protocol::ComparisonOperator::Equal,
                            "true",
                        ),
                        "bool-condition",
                    ),
                    webui_protocol::WebUIFragment::if_cond(
                        webui_protocol::ConditionExpr::predicate(
                            "123",
                            webui_protocol::ComparisonOperator::GreaterThan,
                            "0",
                        ),
                        "number-condition",
                    ),
                ],
            },
        ),
        (
            "bool-condition".to_string(),
            webui_protocol::FragmentList {
                fragments: Vec::new(),
            },
        ),
        (
            "number-condition".to_string(),
            webui_protocol::FragmentList {
                fragments: Vec::new(),
            },
        ),
    ]));

    let schema = generate_schema(&protocol, "index.html", "LiteralKeysState").unwrap();
    assert!(schema["properties"]["true"]["type"].is_array());
    assert!(schema["properties"]["42"]["type"].is_array());
    assert_eq!(schema["properties"]["false"]["type"], "boolean");
    assert_eq!(schema["properties"]["123"]["type"], "number");
    assert_eq!(schema["required"], serde_json::json!(["42", "true"]));
}

#[test]
fn example_state_files_match_generated_schemas() {
    let workspace = workspace_root();
    assert_all_state_examples_are_covered(&workspace);
    let theme_path = workspace.join("packages/webui-examples-theme/tokens.json");

    for example in EXAMPLES {
        let app_root = workspace.join("examples/app").join(example.name);
        let theme = example
            .uses_theme
            .then(|| webui::load_token_file(&theme_path).unwrap());
        let result = build_app(&app_root.join("src"), example.plugin, theme.clone());
        let schema = generate_schema(&result.protocol, "index.html", example.name).unwrap();
        let state_path = app_root.join("data/state.json");
        let mut state: Value =
            serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();

        if let Some(theme) = theme {
            let token_css = webui_tokens::resolve_tokens(&result.protocol.tokens, &theme).unwrap();
            webui_tokens::inject_token_css(&mut state, &token_css.css);
        }
        state
            .as_object_mut()
            .unwrap()
            .insert("basePath".to_string(), Value::String("/".to_string()));

        let state_schema = schema_for_route(&schema, "/");
        let errors = validate_state(state_schema, &state);
        assert!(
            errors.is_empty(),
            "{} state does not match its generated schema:\n{}",
            example.name,
            errors.join("\n")
        );
    }
}

#[test]
fn missing_entry_is_an_error() {
    let protocol = WebUIProtocol::new(std::collections::HashMap::new());
    let error = generate_schema(&protocol, "missing.html", "MissingState").unwrap_err();
    assert!(error.to_string().contains("missing.html"));
}

fn build_fixture_schema(fixture: &str, title: &str) -> Value {
    let app_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/state-schema")
        .join(fixture);
    build_schema(&app_dir, Plugin::WebUI, None, title)
}

fn build_schema(
    app_dir: &Path,
    plugin: Plugin,
    theme: Option<webui::TokenFile>,
    title: &str,
) -> Value {
    let result = build_app(app_dir, plugin, theme);
    generate_schema(&result.protocol, "index.html", title).unwrap()
}

fn build_app(
    app_dir: &Path,
    plugin: Plugin,
    theme: Option<webui::TokenFile>,
) -> webui::BuildResult {
    webui::build(BuildOptions {
        app_dir: app_dir.to_path_buf(),
        plugin: Some(plugin),
        theme,
        ..BuildOptions::default()
    })
    .unwrap()
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn assert_all_state_examples_are_covered(workspace: &Path) {
    let examples_dir = workspace.join("examples/app");
    let discovered: BTreeSet<String> = fs::read_dir(&examples_dir)
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.join("src/index.html").is_file() && path.join("data/state.json").is_file() {
                Some(entry.file_name().to_string_lossy().into_owned())
            } else {
                None
            }
        })
        .collect();
    let expected: BTreeSet<String> = EXAMPLES
        .iter()
        .map(|example| example.name.to_string())
        .collect();
    assert_eq!(discovered, expected);
}

fn schema_for_route<'a>(schema: &'a Value, route: &str) -> &'a Value {
    let Some(routes) = schema.get("x-webui-routes").and_then(Value::as_object) else {
        return schema;
    };
    let schema_ref = routes.get(route).and_then(Value::as_str).unwrap();
    let key = schema_ref.strip_prefix("#/$defs/").unwrap();
    &schema["$defs"][key]
}

fn validate_state(schema: &Value, state: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    let mut pending = vec![(schema, state, "$".to_string())];
    while let Some((current_schema, current_value, path)) = pending.pop() {
        let actual_type = json_type(current_value);
        if !schema_accepts_type(current_schema, actual_type) {
            errors.push(format!(
                "{path}: expected {}, found {actual_type}",
                display_schema_type(current_schema)
            ));
            continue;
        }

        if let (Some(properties), Some(object)) = (
            current_schema.get("properties").and_then(Value::as_object),
            current_value.as_object(),
        ) {
            if let Some(required) = current_schema.get("required").and_then(Value::as_array) {
                for name in required.iter().filter_map(Value::as_str) {
                    if !object.contains_key(name) {
                        errors.push(format!("{path}.{name}: missing required property"));
                    }
                }
            }
            for (name, child_schema) in properties {
                if let Some(child_value) = object.get(name) {
                    pending.push((child_schema, child_value, format!("{path}.{name}")));
                }
            }
        } else if let (Some(item_schema), Some(items)) =
            (current_schema.get("items"), current_value.as_array())
        {
            for (index, item) in items.iter().enumerate() {
                pending.push((item_schema, item, format!("{path}[{index}]")));
            }
        }
    }
    errors
}

fn schema_accepts_type(schema: &Value, actual: &str) -> bool {
    match schema.get("type") {
        Some(Value::String(expected)) => expected == actual,
        Some(Value::Array(expected)) => expected.iter().any(|value| value.as_str() == Some(actual)),
        _ => true,
    }
}

fn display_schema_type(schema: &Value) -> String {
    match schema.get("type") {
        Some(Value::String(expected)) => expected.clone(),
        Some(Value::Array(expected)) => {
            let mut display = String::new();
            for value in expected.iter().filter_map(Value::as_str) {
                if !display.is_empty() {
                    display.push_str(" | ");
                }
                display.push_str(value);
            }
            display
        }
        _ => "any JSON value".to_string(),
    }
}

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn protocol_with_entry_routes(routes: Vec<WebUIFragmentRoute>) -> WebUIProtocol {
    let mut component_ids = BTreeSet::new();
    let mut pending: Vec<&WebUIFragmentRoute> = routes.iter().collect();
    while let Some(route) = pending.pop() {
        component_ids.insert(route.fragment_id.clone());
        pending.extend(&route.children);
    }
    let fragments = routes
        .into_iter()
        .map(webui_protocol::WebUIFragment::route_from)
        .collect();
    let mut records = std::collections::HashMap::from([(
        "index.html".to_string(),
        webui_protocol::FragmentList { fragments },
    )]);
    for component_id in component_ids {
        records.insert(
            component_id,
            webui_protocol::FragmentList {
                fragments: Vec::new(),
            },
        );
    }
    WebUIProtocol::new(records)
}

fn decode_definition_ref(schema_ref: &str) -> String {
    let token = schema_ref.strip_prefix("#/$defs/").unwrap();
    let bytes = token.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex_value(bytes[index + 1]);
            let low = hex_value(bytes[index + 2]);
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded)
        .unwrap()
        .replace("~1", "/")
        .replace("~0", "~")
}

fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => panic!("invalid hex digit in generated schema reference"),
    }
}
