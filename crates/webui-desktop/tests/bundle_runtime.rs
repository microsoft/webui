// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use std::collections::HashMap;
use std::fs;

use prost::Message;
use tempfile::TempDir;
use webui_desktop::{
    BundleIntegrity, DesktopApp, DesktopBundleConfig, DesktopBundleManifest,
    DesktopProtocolRequest, DesktopProtocolResponse, DesktopRuntime, DesktopShellConfig,
    WindowOptions,
};
use webui_protocol::{
    web_ui_fragment::Fragment, FragmentList, WebUIFragment, WebUIFragmentRaw, WebUIFragmentSignal,
    WebUIProtocol,
};

#[path = "support/echo.rs"]
mod echo;

fn bundle(plugin: Option<&str>) -> TempDir {
    let dir = TempDir::new().unwrap();
    let protocol = WebUIProtocol {
        fragments: HashMap::from([(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment {
                        fragment: Some(Fragment::Raw(WebUIFragmentRaw {
                            value: "<html><head></head><body><h1>".to_string(),
                        })),
                    },
                    WebUIFragment {
                        fragment: Some(Fragment::Signal(WebUIFragmentSignal {
                            value: "greeting".to_string(),
                            ..Default::default()
                        })),
                    },
                    WebUIFragment {
                        fragment: Some(Fragment::Raw(WebUIFragmentRaw {
                            value: "</h1></body></html>".to_string(),
                        })),
                    },
                ],
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let manifest = DesktopBundleManifest {
        manifest_version: DesktopBundleManifest::VERSION,
        app_id: "test.runtime-only".to_string(),
        app_name: "Runtime-only fixture".to_string(),
        version: "1.0.0".to_string(),
        publisher: "WebUI".to_string(),
        entry: "index.html".to_string(),
        plugin: plugin.map(str::to_string),
        protocol_path: "protocol.bin".into(),
        state_path: Some("state.json".into()),
        assets_dir: "assets".into(),
        ipc_schema: None,
        window: WindowOptions::default(),
        shell: DesktopShellConfig::default(),
        package_targets: Vec::new(),
        integrity: BundleIntegrity::default(),
    };
    fs::create_dir(dir.path().join("assets")).unwrap();
    fs::write(dir.path().join("protocol.bin"), protocol.encode_to_vec()).unwrap();
    fs::write(
        dir.path().join("manifest.webui-desktop.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(dir.path().join("state.json"), r#"{"greeting":"Bundled"}"#).unwrap();
    fs::write(dir.path().join("assets/app.css"), "body { color: green; }").unwrap();
    dir
}

#[test]
fn loads_precompiled_bundles_with_each_hydration_plugin() {
    for plugin in [
        None,
        Some("fast"),
        Some("fast-v2"),
        Some("fast-v3"),
        Some("webui"),
    ] {
        let dir = bundle(plugin);
        let runtime = DesktopRuntime::from_bundle(dir.path().to_path_buf()).unwrap();
        assert!(runtime.startup_html().contains("Bundled"), "{plugin:?}");
        let response = runtime
            .handle_request(&DesktopProtocolRequest::get("/"))
            .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, runtime.startup_html().as_bytes());
        let asset = runtime
            .handle_request(&DesktopProtocolRequest::get("/app.css"))
            .unwrap();
        assert_eq!(asset.body, b"body { color: green; }");
    }
}

#[test]
fn bundle_config_preserves_host_state_routes_and_ipc() {
    let dir = bundle(Some("webui"));
    let mut config = DesktopBundleConfig::new(dir.path().to_path_buf());
    config.state = Some(serde_json::from_str(r#"{"greeting":"Host"}"#).unwrap());
    config.ipc_registry = echo::registry();
    config
        .api_routes
        .route("/api/hello", |_| {
            Ok(DesktopProtocolResponse::text(200, "Hello"))
        })
        .unwrap();
    let frame = DesktopApp::from_bundle_config(config)
        .unwrap()
        .ipc_options(echo::options())
        .build()
        .unwrap();
    let runtime = &frame.runtime;
    assert!(runtime.startup_html().contains("Host"));
    assert!(!runtime.startup_html().contains("Bundled"));
    assert_eq!(
        runtime
            .handle_request(&DesktopProtocolRequest::get("/api/hello"))
            .unwrap()
            .body,
        b"Hello"
    );
    echo::assert_echo(&frame, b"runtime-only");
}

#[test]
fn preloaded_manifest_needs_no_second_manifest_read() {
    let dir = bundle(None);
    let path = dir.path().join("manifest.webui-desktop.json");
    let manifest = DesktopBundleManifest::load(&path).unwrap();
    fs::remove_file(path).unwrap();
    let runtime = DesktopRuntime::from_bundle_config_and_manifest(
        DesktopBundleConfig::new(dir.path().to_path_buf()),
        manifest,
    )
    .unwrap();
    assert!(runtime.startup_html().contains("Bundled"));
}
