// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use std::collections::HashMap;
use std::fs;

use prost::Message;
use tempfile::TempDir;
use webui_desktop::{
    desktop_ipc_response, BundleIntegrity, DesktopBundleConfig, DesktopBundleManifest,
    DesktopIpcRequest, DesktopIpcResponse, DesktopProtocolRequest, DesktopProtocolResponse,
    DesktopRuntime, DesktopShellConfig, IpcRegistry, WindowOptions, IPC_ENDPOINT, IPC_VERSION,
};
use webui_protocol::{
    web_ui_fragment::Fragment, FragmentList, WebUIFragment, WebUIFragmentRaw, WebUIFragmentSignal,
    WebUIProtocol,
};

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
    config.ipc_registry = IpcRegistry::default();
    config
        .api_routes
        .route("/api/hello", |_| {
            Ok(DesktopProtocolResponse::text(200, "Hello"))
        })
        .unwrap();
    config
        .ipc_registry
        .register("echo", |payload| Ok(payload.to_vec()));
    let runtime = DesktopRuntime::from_bundle_config(config).unwrap();
    assert!(runtime.startup_html().contains("Host"));
    assert!(!runtime.startup_html().contains("Bundled"));
    assert_eq!(
        runtime
            .handle_request(&DesktopProtocolRequest::get("/api/hello"))
            .unwrap()
            .body,
        b"Hello"
    );
    let frame = DesktopIpcRequest {
        version: IPC_VERSION,
        request_id: 7,
        method: "echo".to_string(),
        payload: b"runtime-only".to_vec(),
    }
    .encode_to_vec();
    let response = runtime
        .handle_request(&DesktopProtocolRequest::post(IPC_ENDPOINT, &frame))
        .unwrap();
    let reply = DesktopIpcResponse::decode(response.body.as_slice()).unwrap();
    assert_eq!(reply.request_id, 7);
    assert_eq!(
        reply.result,
        Some(desktop_ipc_response::Result::Payload(
            b"runtime-only".to_vec()
        ))
    );
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
