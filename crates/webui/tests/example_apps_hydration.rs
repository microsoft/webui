// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! End-to-end evidence that projected hydration works on the real example
//! applications shipped in `examples/app/` — the contact-book-manager and the
//! commerce marketplace.
//!
//! Each test compiles the live app source (no cached `.pb`), so any decorator
//! change in those apps is reflected here. The tests assert two things:
//!
//! 1. **Build side** — the aggregated `protocol.hydration_schema` contains the
//!    real `@observable` / `@attr` property names authored in the components,
//!    including the awkward shapes (`@attr({ attribute: '…' }) productTitle`,
//!    definite-assignment `@attr subtotal!: string`, observable arrays).
//! 2. **Runtime side** — rendering with a large server-only state emits a
//!    `#webui-data` bootstrap block that keeps the hydratable keys and projects
//!    the server-only payload out entirely. This is the CPU/byte win: the block
//!    tracks the hydratable surface, not total state size.

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

fn schema_has(protocol: &WebUIProtocol, key: &str) -> bool {
    protocol.hydration_schema.iter().any(|k| k == key)
}

#[test]
fn contact_book_manager_projects_hydration_surface() {
    let protocol = build_example("contact-book-manager");

    // Build side: real `@observable` fields from cb-contact-form.ts et al.
    assert!(
        !protocol.hydration_schema.is_empty(),
        "contact-book hydration schema must not be empty"
    );
    for key in ["firstName", "lastName", "email", "formTitle", "editId"] {
        assert!(
            schema_has(&protocol, key),
            "contact-book schema missing `{key}`: {:?}",
            protocol.hydration_schema
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

    let html = render(&protocol, &state, "/");
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
}

#[test]
fn commerce_projects_hydration_surface() {
    let protocol = build_example("commerce");

    // Build side: covers factory-mapped `@attr({ attribute: 'product-title' })
    // productTitle`, definite-assignment `@attr subtotal!: string`, and the
    // `@observable cartItems!: any[]` array field.
    assert!(
        !protocol.hydration_schema.is_empty(),
        "commerce hydration schema must not be empty"
    );
    for key in ["productTitle", "subtotal", "cartItems", "selectedColor"] {
        assert!(
            schema_has(&protocol, key),
            "commerce schema missing `{key}`: {:?}",
            protocol.hydration_schema
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
