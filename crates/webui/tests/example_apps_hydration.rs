// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! End-to-end evidence that projected hydration works on the real example
//! applications shipped in `examples/app/` — routes, contact-book-manager, and
//! the commerce marketplace.
//!
//! Each test compiles the live app source (no cached `.pb`), so any decorator
//! change in those apps is reflected here. The tests assert two things:
//!
//! 1. **Build side** — per-component `hydration_keys` contain only real
//!    `@observable` / `@attr` property names authored in client components,
//!    while `navigation_keys` retain compiled template roots.
//! 2. **Runtime side** — rendering with a large server-only state emits a
//!    `#webui-data` bootstrap block that keeps keys needed by the active route
//!    and projects inactive-route and server-only payloads out.

use serde_json::{json, Value};
use std::path::PathBuf;
use webui::{
    build, BuildOptions, CssStrategy, Plugin, RenderOptions, ResponseWriter, WebUIHandler,
    WebUIProtocol,
};
use webui_handler::plugin::webui::WebUIHydrationPlugin;

/// Locate `examples/app/<app>/src` relative to this crate's manifest.
fn example_app_dir(app: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join("app")
        .join(app)
        .join("src")
}

/// Compile the live example app into a protocol using the WebUI parser plugin
/// (the plugin owns the `@observable` / `@attr` hydration-surface scan).
fn build_example(app: &str) -> WebUIProtocol {
    let app_dir = example_app_dir(app);
    assert!(
        app_dir.join("index.html").exists(),
        "example `{app}` source not found at {}",
        app_dir.display()
    );
    build(BuildOptions {
        app_dir,
        entry: "index.html".to_string(),
        // Style bakes CSS into fragments so the test needs no external files.
        css: CssStrategy::Style,
        plugin: Some(Plugin::WebUI),
        ..BuildOptions::default()
    })
    .unwrap_or_else(|e| panic!("failed to build `{app}`: {e}"))
    .protocol
}

#[derive(Default)]
struct CaptureWriter {
    output: String,
}

impl ResponseWriter for CaptureWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.output.push_str(content);
        Ok(())
    }
    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

/// Render `state` at `path`. The WebUI hydration plugin is required — the
/// handler only emits the `#webui-data` bootstrap block when a plugin is
/// present, matching the production Node/FFI/WASM hosts.
fn render(protocol: &WebUIProtocol, state: &Value, path: &str) -> String {
    let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
    let mut writer = CaptureWriter::default();
    handler
        .handle(
            protocol,
            state,
            &RenderOptions::new("index.html", path),
            &mut writer,
        )
        .unwrap_or_else(|e| panic!("render failed for `{path}`: {e}"));
    writer.output
}

/// Extract the JSON text inside the `#webui-data` bootstrap `<script>` block.
/// The JSON is `</`-escaped, so the first `>` closes the open tag and the first
/// `</script>` closes the block.
fn webui_data_block(html: &str) -> &str {
    let marker = "id=\"webui-data\"";
    let start = html.find(marker).expect("#webui-data block missing");
    let open = html[start..].find('>').expect("unterminated open tag") + start + 1;
    let close = html[open..].find("</script>").expect("unterminated block") + open;
    &html[open..close]
}

fn webui_data(html: &str) -> Value {
    serde_json::from_str(webui_data_block(html))
        .unwrap_or_else(|error| panic!("#webui-data should be valid JSON: {error}"))
}

fn schema_has(protocol: &WebUIProtocol, key: &str) -> bool {
    protocol.components.values().any(|component| {
        component
            .hydration_keys
            .iter()
            .any(|candidate| candidate == key)
    })
}

#[test]
fn contact_book_manager_projects_hydration_surface() {
    let protocol = build_example("contact-book-manager");

    let cb_app = protocol
        .components
        .get("cb-app")
        .unwrap_or_else(|| panic!("contact-book protocol should contain cb-app"));
    assert_eq!(cb_app.hydration_keys, ["searchQuery", "totalFavorites"]);
    assert_eq!(
        cb_app.navigation_keys,
        [
            "activeGroup",
            "groups",
            "page",
            "searchQuery",
            "totalContacts",
            "totalFavorites",
        ]
    );

    // Build side: real `@observable` fields from cb-contact-form.ts et al.
    let hydration_keys = protocol
        .components
        .values()
        .flat_map(|component| component.hydration_keys.iter())
        .collect::<Vec<_>>();
    assert!(
        !hydration_keys.is_empty(),
        "contact-book hydration keys must not be empty"
    );
    for key in ["firstName", "lastName", "email", "formTitle", "editId"] {
        assert!(
            schema_has(&protocol, key),
            "contact-book hydration keys missing `{key}`: {hydration_keys:?}",
        );
    }

    // Runtime side: a hydratable sentinel survives; a large server-only field
    // (never declared reactive) is projected out of the bootstrap block.
    let server_only = "S".repeat(256 * 1024);
    let state = json!({
        "formTitle": "SENTINEL_HYDRATABLE_cb",
        "page": "dashboard",
        "serverOnlyLedger": server_only,
    });
    assert!(
        !schema_has(&protocol, "serverOnlyLedger"),
        "premise broken: server-only key is in the schema"
    );

    let html = render(&protocol, &state, "/contacts/add");
    let block = webui_data_block(&html);
    assert!(
        block.contains("SENTINEL_HYDRATABLE_cb"),
        "hydratable key was projected out of #webui-data"
    );
    assert!(
        !html.contains("serverOnlyLedger"),
        "server-only key leaked into the render"
    );
    assert!(
        !html.contains(&server_only),
        "server-only blob leaked into the render"
    );

    let dashboard_html = render(&protocol, &state, "/");
    let dashboard_block = webui_data_block(&dashboard_html);
    assert!(
        !dashboard_block.contains("SENTINEL_HYDRATABLE_cb"),
        "inactive contact-form state leaked into the dashboard bootstrap"
    );

    let home_state = json!({
        "groups": ["Work", "Family", "Friends", "Other"],
        "page": "dashboard",
        "recentContacts": [],
        "totalContacts": 15,
        "totalFavorites": 5,
        "totalGroups": 4,
    });
    let home_data = webui_data(&render(&protocol, &home_state, "/"));
    assert_eq!(home_data["state"], json!({ "totalFavorites": 5 }));
}

#[test]
fn routes_example_hydrates_behavior_without_bootstrap_state() {
    let protocol = build_example("routes");
    let routes_app = protocol
        .components
        .get("routes-app")
        .unwrap_or_else(|| panic!("routes protocol should contain routes-app"));

    assert!(routes_app.hydration_keys.is_empty());
    assert_eq!(
        routes_app.navigation_keys,
        ["appTitle", "sectionId", "sections"]
    );
    assert!(
        protocol
            .components
            .values()
            .all(|component| component.hydration_keys.is_empty()),
        "routes example has no @observable or @attr fields"
    );

    let state = json!({
        "appTitle": "Learning Platform",
        "sections": [
            { "icon": "paint", "id": "frontend", "name": "Frontend" },
            { "icon": "gear", "id": "backend", "name": "Backend" },
        ],
    });
    let full_data = webui_data(&render(&protocol, &state, "/"));
    assert_eq!(full_data["state"], json!({}));

    let partial =
        webui_handler::route_handler::render_partial(&protocol, state, "index.html", "/", "")
            .unwrap_or_else(|error| panic!("routes partial should render: {error}"));
    assert_eq!(partial["state"]["appTitle"], "Learning Platform");
    assert_eq!(
        partial["state"]["sections"].as_array().map_or(0, Vec::len),
        2
    );
}

#[test]
fn commerce_projects_hydration_surface() {
    let protocol = build_example("commerce");

    // Build side: covers factory-mapped `@attr({ attribute: 'product-title' })
    // productTitle`, definite-assignment `@attr subtotal!: string`, and the
    // `@observable cartItems!: any[]` array field.
    let hydration_keys = protocol
        .components
        .values()
        .flat_map(|component| component.hydration_keys.iter())
        .collect::<Vec<_>>();
    assert!(
        !hydration_keys.is_empty(),
        "commerce hydration keys must not be empty"
    );
    for key in ["productTitle", "subtotal", "cartItems", "selectedColor"] {
        assert!(
            schema_has(&protocol, key),
            "commerce hydration keys missing `{key}`: {hydration_keys:?}",
        );
    }

    let server_only = "C".repeat(256 * 1024);
    let state = json!({
        "subtotal": "SENTINEL_HYDRATABLE_mp",
        "storeName": "Test Store",
        "serverProductCatalog": server_only,
    });
    assert!(
        !schema_has(&protocol, "serverProductCatalog"),
        "premise broken: server-only key is in the schema"
    );

    let html = render(&protocol, &state, "/");
    let block = webui_data_block(&html);
    assert!(
        block.contains("SENTINEL_HYDRATABLE_mp"),
        "hydratable key was projected out of #webui-data"
    );
    assert!(
        !html.contains("serverProductCatalog"),
        "server-only key leaked into the render"
    );
    assert!(
        !html.contains(&server_only),
        "server-only blob leaked into the render"
    );
}
