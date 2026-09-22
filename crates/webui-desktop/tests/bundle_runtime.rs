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
#[cfg(feature = "application-ipc")]
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
        assert!(asset.body.as_bytes().is_none());
        assert_eq!(asset.body.into_bytes().unwrap(), b"body { color: green; }");
    }
}

fn index_asset(dir: &TempDir, name: &str) {
    let path = dir.path().join("manifest.webui-desktop.json");
    let mut manifest = DesktopBundleManifest::load(&path).unwrap();
    manifest.integrity.assets.push(webui_desktop::BundleAsset {
        path: format!("assets/{name}"),
        sha256: String::new(),
        size_bytes: 6,
    });
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
}

#[test]
fn indexed_assets_decode_url_paths_once_and_preserve_literal_manifest_names() {
    let dir = bundle(None);
    for (name, url) in [
        ("logo mark.svg", "/logo%20mark.svg"),
        ("percent%20.svg", "/percent%2520.svg"),
        ("question?.svg", "/question%3F.svg"),
    ] {
        if cfg!(windows) && name.contains('?') {
            continue; // Windows does not allow question marks in filenames.
        }
        fs::write(dir.path().join("assets").join(name), b"secret").unwrap();
        index_asset(&dir, name);
        let runtime = DesktopRuntime::from_bundle(dir.path().to_path_buf()).unwrap();
        let response = runtime
            .handle_request(&DesktopProtocolRequest::get(url))
            .unwrap();
        assert_eq!(response.status, 200, "{url}");
        assert_eq!(response.body.into_bytes().unwrap(), b"secret");
    }
}

#[cfg(unix)]
#[test]
fn indexed_assets_cannot_follow_escaping_symlinks_before_or_after_load() {
    use std::os::unix::fs::symlink;
    let dir = bundle(None);
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret"), b"secret").unwrap();
    let path = dir.path().join("assets/secret");
    fs::write(&path, b"inside").unwrap();
    index_asset(&dir, "secret");
    let runtime = DesktopRuntime::from_bundle(dir.path().to_path_buf()).unwrap();
    fs::remove_file(&path).unwrap();
    symlink(outside.path().join("secret"), &path).unwrap();
    assert!(runtime
        .handle_request(&DesktopProtocolRequest::get("/secret"))
        .is_err());
    assert!(DesktopRuntime::from_bundle(dir.path().to_path_buf()).is_err());
}

#[test]
fn provider_backed_root_reload_is_fresh_and_propagates_failure() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let dir = bundle(None);
    let counter = Arc::new(AtomicUsize::new(0));
    let mut config = DesktopBundleConfig::new(dir.path().to_path_buf());
    let calls = Arc::clone(&counter);
    config
        .route_state
        .route("/", move |_| {
            let count = calls.fetch_add(1, Ordering::SeqCst);
            if count == 3 {
                return Err(webui_desktop::DesktopError::InvalidAssetPath {
                    path: "provider failure".into(),
                });
            }
            Ok(serde_json::json!({"greeting": format!("visit {count}")}))
        })
        .unwrap();
    let runtime = DesktopRuntime::from_bundle_config(config).unwrap();
    assert!(runtime.startup_html().contains("visit 0"));
    for (path, expected) in [("/", "visit 1"), ("/index.html", "visit 2")] {
        let response = runtime
            .handle_request(&DesktopProtocolRequest::get(path))
            .unwrap();
        assert!(std::str::from_utf8(response.body.as_bytes().unwrap())
            .unwrap()
            .contains(expected));
    }
    assert!(runtime
        .handle_request(&DesktopProtocolRequest::get("/"))
        .is_err());
}

#[test]
fn bundle_config_preserves_host_state_routes_and_ipc() {
    let dir = bundle(Some("webui"));
    let mut config = DesktopBundleConfig::new(dir.path().to_path_buf());
    config.state = Some(serde_json::from_str(r#"{"greeting":"Host"}"#).unwrap());
    #[cfg(feature = "application-ipc")]
    {
        config.ipc_registry = echo::registry();
    }
    config
        .api_routes
        .route("/api/hello", |_| {
            Ok(DesktopProtocolResponse::text(200, "Hello"))
        })
        .unwrap();
    let builder = DesktopApp::from_bundle_config(config).unwrap();
    #[cfg(feature = "application-ipc")]
    let builder = builder.ipc_options(echo::options());
    let frame = builder.build().unwrap();
    let runtime = frame.runtime();
    assert!(runtime.startup_html().contains("Host"));
    assert!(!runtime.startup_html().contains("Bundled"));
    assert_eq!(
        runtime
            .handle_request(&DesktopProtocolRequest::get("/api/hello"))
            .unwrap()
            .body,
        b"Hello"
    );
    #[cfg(feature = "application-ipc")]
    echo::assert_echo(&frame, b"runtime-only");
}

#[test]
#[cfg(not(feature = "application-ipc"))]
fn no_ipc_runtime_has_no_embedded_assets_or_reserved_endpoints() {
    let dir = bundle(None);
    let runtime = DesktopRuntime::from_bundle(dir.path().to_path_buf()).unwrap();
    for path in [
        "/_webui/ipc",
        "/_webui/ipc/outbound",
        "/_webui/ipc/runtime.js",
        "/_webui/ipc/bootstrap.js",
    ] {
        let response = runtime
            .handle_request(&DesktopProtocolRequest::get(path))
            .unwrap();
        assert_eq!(response.status, 404, "{path}");
        let response = runtime
            .handle_request(&DesktopProtocolRequest::post(path, b"no IPC"))
            .unwrap();
        assert_eq!(response.status, 404, "{path}");
    }
    let mut config = DesktopBundleConfig::new(dir.path().to_path_buf());
    config
        .api_routes
        .route("/_webui/ipc", |_| {
            Ok(DesktopProtocolResponse::text(200, "Application-owned API"))
        })
        .unwrap();
    let runtime = DesktopRuntime::from_bundle_config(config).unwrap();
    assert_eq!(
        runtime
            .handle_request(&DesktopProtocolRequest::get("/_webui/ipc"))
            .unwrap()
            .body,
        b"Application-owned API"
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
