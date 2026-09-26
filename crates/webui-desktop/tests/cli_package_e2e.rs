// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;
use webui_desktop::{DesktopBundleManifest, DesktopRuntime, TitlebarStyle};

fn write_file(root: &Path, path: &str, content: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn desktop_cli(args: &[&str]) {
    let output = Command::new(env!("CARGO_BIN_EXE_webui-desktop"))
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "desktop CLI failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn hydrated_app_without_desktop_settings_fails_instead_of_downgrading() {
    let dir = TempDir::new().unwrap();
    let app = dir.path().join("app");
    let package = dir.path().join("packages");
    write_file(&app, "package.json", r#"{"name":"hydrated-app"}"#);
    write_file(
        &app,
        "src/index.html",
        "<!doctype html><html><body>Hydrated</body></html>",
    );
    write_file(&app, "dist/webui-projection.json", "{}");
    let output = Command::new(env!("CARGO_BIN_EXE_webui-desktop"))
        .args([
            "package",
            app.to_str().unwrap(),
            "--out",
            package.to_str().unwrap(),
            "--runner",
            env!("CARGO_BIN_EXE_webui-desktop"),
            "--no-web-build",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("webui-desktop.json"), "{error}");
    assert!(error.contains("--plugin webui"), "{error}");
    assert!(!package.exists());
}

#[test]
fn init_then_package_keeps_source_identity_and_title() {
    let dir = TempDir::new().unwrap();
    let app = dir.path().join("app");
    let bundle = dir.path().join("bundle");
    let package = dir.path().join("packages");
    let cli = env!("CARGO_BIN_EXE_webui-desktop");
    desktop_cli(&["init", app.to_str().unwrap()]);
    assert!(!fs::read_to_string(app.join("package.json"))
        .unwrap()
        .contains("webuiDesktop"));
    let source = fs::read_to_string(app.join("desktop/src/main.rs")).unwrap();
    assert!(source.contains("webui-desktop.json"));
    desktop_cli(&[
        "package",
        app.to_str().unwrap(),
        "--out",
        package.to_str().unwrap(),
        "--bundle-out",
        bundle.to_str().unwrap(),
        "--runner",
        cli,
        "--no-web-build",
    ]);

    let settings: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(app.join("webui-desktop.json")).unwrap()).unwrap();
    let manifest =
        DesktopBundleManifest::load(&bundle.join("manifest.webui-desktop.json")).unwrap();
    assert_eq!(manifest.app_id, settings["appId"]);
    assert_eq!(manifest.app_name, settings["appName"]);
    assert_eq!(manifest.window.title, settings["title"]);
    assert_eq!(manifest.app_id, "com.example.webui.desktop");
    assert_eq!(manifest.window.title, "WebUI Desktop App");
    assert!(package
        .join("WebUI-Desktop-App.app/Contents/Resources/webui/protocol.bin")
        .is_file());
}

#[test]
fn standalone_config_packages_hydrated_assets_and_window_without_package_metadata() {
    let dir = TempDir::new().unwrap();
    let app = dir.path().join("app");
    let bundle = dir.path().join("bundle");
    let package = dir.path().join("packages");
    write_file(&app, "package.json", r#"{"name":"demo","version":"1.2.3"}"#);
    write_file(
        &app,
        "webui-desktop.json",
        r##"{
          "app": "src",
          "state": "data/state.json",
          "assets": "dist",
          "icon": "desktop/icon.icns",
          "plugin": "webui",
          "theme": "themes/tokens.json",
          "appId": "com.example.hydrated",
          "appName": "Hydrated App",
          "title": "Hydrated Window",
          "width": 960,
          "height": 700,
          "titlebar": {"style": "overlay", "height": 48},
          "background": "#f8fafc",
          "rememberState": true,
          "devtools": true
        }"##,
    );
    write_file(
        &app,
        "src/index.html",
        "<!doctype html><html><head><style>:root{/*{{{tokens.light}}}*/}body{color:var(--brand)}</style></head><body><demo-widget></demo-widget></body></html>",
    );
    write_file(
        &app,
        "src/demo-widget/demo-widget.html",
        "<for each=\"item in items\"><span>{{item}}</span></for>",
    );
    write_file(&app, "data/state.json", r#"{"items":["Hydrated!"]}"#);
    write_file(
        &app,
        "themes/tokens.json",
        r##"{"themes":{"light":{"brand":"#0078d4"}}}"##,
    );
    write_file(&app, "dist/app.js", "export {};");
    write_file(&app, "desktop/icon.icns", "icns");
    desktop_cli(&[
        "package",
        app.to_str().unwrap(),
        "--out",
        package.to_str().unwrap(),
        "--bundle-out",
        bundle.to_str().unwrap(),
        "--runner",
        env!("CARGO_BIN_EXE_webui-desktop"),
        "--no-web-build",
    ]);

    let manifest =
        DesktopBundleManifest::load(&bundle.join("manifest.webui-desktop.json")).unwrap();
    assert_eq!(manifest.app_id, "com.example.hydrated");
    assert_eq!(manifest.app_name, "Hydrated App");
    assert_eq!(manifest.window.title, "Hydrated Window");
    assert_eq!((manifest.window.width, manifest.window.height), (960, 700));
    assert_eq!(
        manifest.window.titlebar,
        TitlebarStyle::Overlay { height: 48 }
    );
    assert!(manifest.window.remember_state);
    assert!(manifest.window.devtools);
    assert_eq!(manifest.window.background.unwrap().to_string(), "#f8fafc");
    assert_eq!(manifest.plugin.as_deref(), Some("webui"));
    assert!(manifest.shell.icon_path.is_some());
    let runtime = DesktopRuntime::from_bundle(bundle).unwrap();
    assert!(runtime.startup_html().contains("Hydrated!"));
    assert!(runtime.startup_html().contains("<!--wr-->"));
    assert!(runtime.startup_html().contains("--brand: #0078d4;"));
    let contents = package.join("Hydrated-App.app/Contents");
    assert_eq!(
        fs::read(contents.join("Resources/AppIcon.icns")).unwrap(),
        b"icns"
    );
    assert!(contents.join("Resources/webui/assets/app.js").is_file());
    assert!(contents.join("Resources/webui/protocol.bin").is_file());

    let overridden_bundle = dir.path().join("overridden-bundle");
    let overridden_package = dir.path().join("overridden-package");
    desktop_cli(&[
        "package",
        app.to_str().unwrap(),
        "--out",
        overridden_package.to_str().unwrap(),
        "--bundle-out",
        overridden_bundle.to_str().unwrap(),
        "--runner",
        env!("CARGO_BIN_EXE_webui-desktop"),
        "--no-web-build",
        "--app-id",
        "com.example.explicit",
        "--title",
        "Explicit Title",
        "--remember-state=false",
    ]);
    let overridden =
        DesktopBundleManifest::load(&overridden_bundle.join("manifest.webui-desktop.json"))
            .unwrap();
    assert_eq!(overridden.app_id, "com.example.explicit");
    assert_eq!(overridden.window.title, "Explicit Title");
    assert!(!overridden.window.remember_state);
    assert_eq!(overridden.plugin.as_deref(), Some("webui"));
}
