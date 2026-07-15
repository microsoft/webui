// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! End-to-end evidence that projection is strictly opt-in for real examples.
//!
//! These examples do not yet run their client bundler with a projection
//! adapter. Rust therefore performs no TypeScript inference: initial and
//! scripted navigation state remain full until the example-conversion phase
//! supplies real bundler manifests.

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

/// Compile the live example app without projection manifests.
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

#[test]
fn contact_book_manager_without_manifest_preserves_full_state() {
    let protocol = build_example("contact-book-manager");

    assert_eq!(
        protocol.initial_state_strategy,
        webui_protocol::InitialStateStrategy::Full as i32
    );
    let cb_app = protocol
        .components
        .get("cb-app")
        .unwrap_or_else(|| panic!("contact-book protocol should contain cb-app"));
    assert_eq!(
        cb_app.hydration_mode,
        webui_protocol::StateProjectionMode::All as i32
    );
    assert_eq!(
        cb_app.navigation_mode,
        webui_protocol::StateProjectionMode::All as i32
    );
    assert!(cb_app.hydration_keys.is_empty());
    assert!(cb_app.navigation_keys.is_empty());

    let server_only = "S".repeat(256 * 1024);
    let state = json!({
        "formTitle": "SENTINEL_HYDRATABLE_cb",
        "page": "dashboard",
        "serverOnlyLedger": server_only,
    });

    let html = render(&protocol, &state, "/contacts/add");
    let block = webui_data_block(&html);
    assert!(block.contains("SENTINEL_HYDRATABLE_cb"));
    assert!(html.contains("serverOnlyLedger"));
    assert!(html.contains(&server_only));

    let home_state = json!({
        "groups": ["Work", "Family", "Friends", "Other"],
        "page": "dashboard",
        "recentContacts": [],
        "totalContacts": 15,
        "totalFavorites": 5,
        "totalGroups": 4,
    });
    let home_data = webui_data(&render(&protocol, &home_state, "/"));
    assert_eq!(home_data["state"], home_state);
}

#[test]
fn routes_example_without_manifest_preserves_full_state() {
    let protocol = build_example("routes");
    assert_eq!(
        protocol.initial_state_strategy,
        webui_protocol::InitialStateStrategy::Full as i32
    );
    let routes_app = protocol
        .components
        .get("routes-app")
        .unwrap_or_else(|| panic!("routes protocol should contain routes-app"));

    assert_eq!(
        routes_app.hydration_mode,
        webui_protocol::StateProjectionMode::All as i32
    );
    assert_eq!(
        routes_app.navigation_mode,
        webui_protocol::StateProjectionMode::All as i32
    );
    assert!(routes_app.hydration_keys.is_empty());
    assert!(routes_app.navigation_keys.is_empty());

    let state = json!({
        "appTitle": "Learning Platform",
        "sections": [
            { "icon": "paint", "id": "frontend", "name": "Frontend" },
            { "icon": "gear", "id": "backend", "name": "Backend" },
        ],
    });
    let full_data = webui_data(&render(&protocol, &state, "/"));
    assert_eq!(full_data["state"], state);

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
fn commerce_without_manifest_preserves_full_state() {
    let protocol = build_example("commerce");

    assert_eq!(
        protocol.initial_state_strategy,
        webui_protocol::InitialStateStrategy::Full as i32
    );

    let server_only = "C".repeat(256 * 1024);
    let state = json!({
        "subtotal": "SENTINEL_HYDRATABLE_mp",
        "storeName": "Test Store",
        "serverProductCatalog": server_only,
    });

    let html = render(&protocol, &state, "/");
    let block = webui_data_block(&html);
    assert!(block.contains("SENTINEL_HYDRATABLE_mp"));
    assert!(html.contains("serverProductCatalog"));
    assert!(html.contains(&server_only));
}
