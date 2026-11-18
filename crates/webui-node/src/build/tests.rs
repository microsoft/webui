// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;
use std::collections::{BTreeMap, HashMap};
use webui_protocol::projection_manifest::{
    ProjectionAdapter, ProjectionComponent, ProjectionManifest, ProjectionProducer, PRODUCER_NAME,
    SCHEMA_ID,
};
use webui_protocol::WebUIProtocol;

fn inspect(protocol_data: Buffer) -> Result<String, webui::WebUIError> {
    webui::inspect_bytes(&protocol_data)
}

fn projection_manifest_json() -> String {
    const EMPTY_SHA256: &str =
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let mut manifest = ProjectionManifest {
        schema: SCHEMA_ID.to_string(),
        producer: ProjectionProducer {
            name: PRODUCER_NAME.to_string(),
            version: "0.0.18".to_string(),
        },
        adapter: ProjectionAdapter {
            name: "test".to_string(),
            bundler: "test@1.0.0".to_string(),
        },
        root: ".".to_string(),
        analysis_hash: format!("sha256:{}", "1".repeat(64)),
        build_id: String::new(),
        inputs: BTreeMap::from([("demo-card.ts".to_string(), EMPTY_SHA256.to_string())]),
        outputs: BTreeMap::from([("bundle.js".to_string(), EMPTY_SHA256.to_string())]),
        components: BTreeMap::from([(
            "demo-card".to_string(),
            ProjectionComponent {
                module: "demo-card.ts".to_string(),
                outputs: vec!["bundle.js".to_string()],
                hydration_keys: vec!["name".to_string()],
                navigation_keys: vec!["label".to_string(), "name".to_string()],
            },
        )]),
        entry_closures: BTreeMap::new(),
    };
    manifest.build_id = manifest.compute_build_id();
    serde_json::to_string(&manifest).unwrap()
}

fn projection_build_options(app_dir: &std::path::Path) -> JsBuildOptions {
    JsBuildOptions {
        app_dir: app_dir.to_string_lossy().to_string(),
        entry: None,
        css: None,
        dom: None,
        css_bundle: None,
        plugin: Some("webui".to_string()),
        components: None,
        component_asset_roots: None,
        metafile: None,
        css_file_name_template: None,
        asset_base: None,
        component_modules: None,
        modules_out: None,
        legal_comments: None,
        theme: None,
        projection_manifests: None,
        projection_manifest_objects: None,
    }
}

#[test]
fn prepared_build_retains_snapshot_and_rejects_reuse() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::TempDir::new()?;
    std::fs::write(dir.path().join("index.html"), "<h1>Retained</h1>")?;
    let mut prepared = prepare_build(projection_build_options(dir.path()))?;
    assert!(prepared.lazy_scripts_module()?.is_none());
    assert!(prepared.component_asset_files()?.is_empty());
    assert!(prepared.lazy_pack_files()?.is_empty());
    prepared.write_modules()?;
    std::fs::write(dir.path().join("index.html"), "<h1>Changed</h1>")?;
    let finish_options = || JsFinishBuildOptions {
        projection_manifests: None,
        projection_manifest_objects: None,
    };
    let result = prepared.finish(finish_options())?;
    let json = inspect(result.protocol)?;
    assert!(json.contains("Retained"));
    assert!(!json.contains("Changed"));
    assert!(prepared.lazy_scripts_module().is_err());
    assert!(prepared.component_asset_files().is_err());
    assert!(prepared.lazy_pack_files().is_err());
    assert!(prepared.write_modules().is_err());
    let error = prepared
        .finish(finish_options())
        .err()
        .ok_or("prepared build was consumed twice")?;
    assert_eq!(error.status, napi::Status::GenericFailure);
    assert!(error.reason.contains("already been consumed"));
    Ok(())
}

#[test]
fn prepared_build_validates_projection_and_consumes_failed_finish(
) -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::TempDir::new()?;
    std::fs::write(dir.path().join("index.html"), "<demo-card></demo-card>")?;
    std::fs::write(dir.path().join("demo-card.html"), "<p>{{name}}</p>")?;
    std::fs::write(dir.path().join("demo-card.ts"), "")?;
    std::fs::write(dir.path().join("bundle.js"), "")?;
    let options = || JsFinishBuildOptions {
        projection_manifests: None,
        projection_manifest_objects: Some(vec![JsProjectionManifest {
            path: dir
                .path()
                .join("projection.json")
                .to_string_lossy()
                .to_string(),
            json: projection_manifest_json(),
        }]),
    };

    let mut prepared = prepare_build(projection_build_options(dir.path()))?;
    let result = prepared.finish(options())?;
    let protocol = WebUIProtocol::from_protobuf(&result.protocol)?;
    assert_eq!(protocol.components["demo-card"].hydration_keys, ["name"]);
    assert_eq!(
        protocol.components["demo-card"].navigation_keys,
        ["label", "name"]
    );

    let mut stale = prepare_build(projection_build_options(dir.path()))?;
    std::fs::write(dir.path().join("bundle.js"), "changed")?;
    let error = stale
        .finish(options())
        .err()
        .ok_or("stale projection was accepted")?;
    assert!(error.reason.contains("PROJ-"), "{error}");
    assert!(stale.lazy_pack_files().is_err());
    assert!(stale.finish(options()).is_err());
    Ok(())
}

#[test]
fn test_build_accepts_projection_paths_and_inline_objects() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("index.html"), "<demo-card></demo-card>").unwrap();
    std::fs::write(dir.path().join("demo-card.html"), "<p>{{name}}</p>").unwrap();
    std::fs::write(dir.path().join("demo-card.ts"), "").unwrap();
    std::fs::write(dir.path().join("bundle.js"), "").unwrap();
    let manifest_path = dir.path().join("projection.json");
    let json = projection_manifest_json();
    std::fs::write(&manifest_path, &json).unwrap();

    let mut path_options = projection_build_options(dir.path());
    path_options.projection_manifests = Some(vec![manifest_path.to_string_lossy().to_string()]);
    let path_result = build(path_options).unwrap();
    let path_protocol = WebUIProtocol::from_protobuf(&path_result.protocol).unwrap();
    assert_eq!(
        path_protocol.components["demo-card"].hydration_keys,
        ["name"]
    );

    std::fs::remove_file(&manifest_path).unwrap();
    let mut inline_options = projection_build_options(dir.path());
    inline_options.projection_manifest_objects = Some(vec![JsProjectionManifest {
        path: manifest_path.to_string_lossy().to_string(),
        json,
    }]);
    let inline_result = build(inline_options).unwrap();
    let inline_protocol = WebUIProtocol::from_protobuf(&inline_result.protocol).unwrap();
    assert_eq!(
        inline_protocol.components["demo-card"].navigation_keys,
        ["label", "name"]
    );
}

#[test]
fn test_build_simple_app() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("index.html"), "<h1>Hello</h1>").unwrap();

    let options = JsBuildOptions {
        app_dir: dir.path().to_string_lossy().to_string(),
        entry: None,
        css: None,
        dom: None,
        css_bundle: None,
        plugin: None,
        components: None,
        component_asset_roots: None,
        metafile: None,
        css_file_name_template: None,
        asset_base: None,
        component_modules: None,
        modules_out: None,
        legal_comments: None,
        theme: None,
        projection_manifests: None,
        projection_manifest_objects: None,
    };

    let result = build(options).unwrap();
    assert!(!result.protocol.is_empty());
    assert!(result.stats.fragment_count > 0);
    assert!(result.stats.protocol_size_bytes > 0);
    assert!(result.stats.duration_ms >= 0.0);
}

#[test]
fn test_build_with_custom_entry() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("page.html"), "<p>Custom</p>").unwrap();

    let options = JsBuildOptions {
        app_dir: dir.path().to_string_lossy().to_string(),
        entry: Some("page.html".to_string()),
        css: None,
        dom: None,
        css_bundle: None,
        plugin: None,
        components: None,
        component_asset_roots: None,
        metafile: None,
        css_file_name_template: None,
        asset_base: None,
        component_modules: None,
        modules_out: None,
        legal_comments: None,
        theme: None,
        projection_manifests: None,
        projection_manifest_objects: None,
    };

    let result = build(options).unwrap();
    assert!(!result.protocol.is_empty());
}

#[test]
fn test_build_missing_app_dir() {
    let options = JsBuildOptions {
        app_dir: "/nonexistent/path".to_string(),
        entry: None,
        css: None,
        dom: None,
        css_bundle: None,
        plugin: None,
        components: None,
        component_asset_roots: None,
        metafile: None,
        css_file_name_template: None,
        asset_base: None,
        component_modules: None,
        modules_out: None,
        legal_comments: None,
        theme: None,
        projection_manifests: None,
        projection_manifest_objects: None,
    };

    let result = build(options);
    assert!(result.is_err());
}

#[test]
fn test_build_invalid_css() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("index.html"), "<h1>Hello</h1>").unwrap();

    let options = JsBuildOptions {
        app_dir: dir.path().to_string_lossy().to_string(),
        entry: None,
        css: Some("bogus".to_string()),
        dom: None,
        css_bundle: None,
        plugin: None,
        components: None,
        component_asset_roots: None,
        metafile: None,
        css_file_name_template: None,
        asset_base: None,
        component_modules: None,
        modules_out: None,
        legal_comments: None,
        theme: None,
        projection_manifests: None,
        projection_manifest_objects: None,
    };

    let result = build(options);
    assert!(result.is_err());
}

#[test]
fn test_build_invalid_dom_strategy() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("index.html"), "<h1>Hello</h1>").unwrap();

    let options = JsBuildOptions {
        app_dir: dir.path().to_string_lossy().to_string(),
        entry: None,
        css: None,
        dom: Some("closed".to_string()),
        css_bundle: None,
        plugin: None,
        components: None,
        component_asset_roots: None,
        metafile: None,
        css_file_name_template: None,
        asset_base: None,
        component_modules: None,
        modules_out: None,
        legal_comments: None,
        theme: None,
        projection_manifests: None,
        projection_manifest_objects: None,
    };

    assert!(build(options).is_err());
}

#[test]
fn test_build_with_components_css() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("index.html"), "<my-card>Hello</my-card>").unwrap();
    std::fs::write(dir.path().join("my-card.html"), "<div>content</div>").unwrap();
    std::fs::write(dir.path().join("my-card.css"), ".card { color: red; }").unwrap();

    let options = JsBuildOptions {
        app_dir: dir.path().to_string_lossy().to_string(),
        entry: None,
        css: Some("link".to_string()),
        dom: None,
        css_bundle: None,
        plugin: None,
        components: None,
        component_asset_roots: None,
        metafile: None,
        css_file_name_template: None,
        asset_base: None,
        component_modules: None,
        modules_out: None,
        legal_comments: None,
        theme: None,
        projection_manifests: None,
        projection_manifest_objects: None,
    };

    let result = build(options).unwrap();
    // css_files is flattened: [filename, content, filename, content, ...]
    assert_eq!(result.css_files.len(), 2);
    assert_eq!(result.css_files[0], "my-card.css");
    assert!(result.css_files[1].contains("color: red"));
    assert_eq!(result.stats.css_file_count, 1);
}

#[test]
fn test_build_with_theme_missing_token_errors() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("index.html"), "<my-card></my-card>").unwrap();
    std::fs::write(dir.path().join("my-card.html"), "<div>Card</div>").unwrap();
    std::fs::write(
        dir.path().join("my-card.css"),
        ":host { --token-a: red; --foo-bar: var(--token-a, var(--token-b, var(--token-c))); }",
    )
    .unwrap();
    let theme_path = dir.path().join("theme.json");
    std::fs::write(&theme_path, r#"{"themes":{"light":{"token-b":"green"}}}"#).unwrap();

    let options = JsBuildOptions {
        app_dir: dir.path().to_string_lossy().to_string(),
        entry: None,
        css: Some("link".to_string()),
        dom: None,
        css_bundle: None,
        plugin: None,
        components: None,
        component_asset_roots: None,
        metafile: None,
        css_file_name_template: None,
        asset_base: None,
        component_modules: None,
        modules_out: None,
        legal_comments: None,
        theme: Some(theme_path.to_string_lossy().to_string()),
        projection_manifests: None,
        projection_manifest_objects: None,
    };

    let Err(err) = build(options) else {
        panic!("missing theme token must fail");
    };
    let message = err.to_string();
    assert!(message.contains("missing-theme-token"), "msg: {message}");
    assert!(message.contains("--token-c"), "msg: {message}");
}

#[test]
fn test_build_reports_lazy_packs_and_the_generated_scripts_module() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("index.html"), "<app-shell></app-shell>").unwrap();
    std::fs::write(
        dir.path().join("app-shell.html"),
        r#"<if condition="open" lazy="eager"><settings-panel></settings-panel></if>"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("settings-panel.html"), "<p>Settings</p>").unwrap();
    std::fs::write(dir.path().join("settings-panel.js"), "export {};").unwrap();
    std::fs::write(dir.path().join("settings-panel.css"), "p { margin: 0; }").unwrap();

    let options = |metafile: bool, modules: Option<HashMap<String, String>>| JsBuildOptions {
        app_dir: dir.path().to_string_lossy().to_string(),
        entry: None,
        css: None,
        dom: None,
        css_bundle: None,
        plugin: Some("webui".to_string()),
        components: None,
        component_asset_roots: None,
        metafile: Some(metafile),
        css_file_name_template: None,
        asset_base: None,
        component_modules: modules,
        modules_out: Some(dir.path().join("generated").to_string_lossy().to_string()),
        legal_comments: None,
        theme: None,
        projection_manifests: None,
        projection_manifest_objects: None,
    };

    let result = build(options(true, None)).unwrap();
    let [pack] = result.lazy_pack_files.as_slice() else {
        panic!("expected one lazy pack");
    };
    assert_eq!(pack.region, "app-shell::lazy:@0");
    assert_eq!(pack.owner, "app-shell");
    assert_eq!(pack.trigger, "eager");
    assert_eq!(pack.path, format!("_webui/packs/{}", pack.file_name));
    assert!(pack.file_name.contains(&pack.hash));
    assert_eq!(pack.bytes as usize, pack.content.len());
    assert!(pack.brotli_bytes.is_some_and(|bytes| bytes > 0));
    assert_eq!(pack.scripts, ["../settings-panel.js"]);
    assert_eq!(pack.stylesheets, ["settings-panel.css"]);
    let module = result.lazy_scripts_module.as_ref().unwrap();
    assert_eq!(module.file_name, "lazy.webui.js");
    assert!(module
        .content
        .contains(r#""settings-panel": () => import("../settings-panel.js"),"#));
    assert_eq!(module.declaration.file_name, "lazy.webui.d.ts");
    assert_eq!(
        module.declaration.content,
        concat!(
            "// Generated by WebUI. Do not edit.\n",
            "import '@microsoft/webui-framework/lazy.js';\n",
        )
    );
    let metafile: serde_json::Value =
        serde_json::from_str(result.metafile.as_deref().unwrap()).unwrap();
    assert_eq!(metafile["lazyPacks"][0]["path"], pack.path.as_str());
    assert_eq!(
        metafile["lazyPacks"][0]["stylesheets"],
        serde_json::json!(["settings-panel.css"])
    );
    let mut inline = options(false, None);
    inline.css = Some("style".to_string());
    let inline = build(inline).unwrap();
    assert!(
        inline.lazy_pack_files[0].stylesheets.is_empty(),
        "style packs carry CSS text"
    );

    let mapped = build(options(
        false,
        Some(HashMap::from([(
            "settings-panel".to_string(),
            "@contoso/settings.js".to_string(),
        )])),
    ))
    .unwrap();
    assert_eq!(mapped.lazy_pack_files[0].brotli_bytes, None);
    assert_eq!(mapped.lazy_pack_files[0].scripts, ["@contoso/settings.js"]);
    assert!(mapped.metafile.is_none());

    let invalid = build(options(
        false,
        Some(HashMap::from([(
            "settings-panel".to_string(),
            "./has space.js".to_string(),
        )])),
    ));
    let Err(error) = invalid else {
        panic!("invalid component module must fail");
    };
    assert!(
        error.to_string().contains("invalid-component-module"),
        "{error}"
    );
}

#[test]
fn test_build_returns_component_asset_files() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("index.html"), "<app-shell></app-shell>").unwrap();
    std::fs::write(dir.path().join("app-shell.html"), "<div></div>").unwrap();
    std::fs::write(dir.path().join("lazy-panel.html"), "<p>{{title}}</p>").unwrap();
    std::fs::write(dir.path().join("lazy-panel.ts"), "export {};").unwrap();

    let options = JsBuildOptions {
        app_dir: dir.path().to_string_lossy().to_string(),
        entry: None,
        css: Some("link".to_string()),
        dom: None,
        css_bundle: None,
        plugin: Some("webui".to_string()),
        components: None,
        component_asset_roots: Some(vec!["lazy-panel".to_string()]),
        metafile: Some(true),
        css_file_name_template: None,
        asset_base: None,
        component_modules: None,
        modules_out: None,
        legal_comments: None,
        theme: None,
        projection_manifests: None,
        projection_manifest_objects: None,
    };

    let result = build(options).unwrap();

    assert_eq!(result.component_asset_files.len(), 4);
    assert_eq!(result.component_asset_files[0], "lazy-panel.webui.js");
    assert!(result.component_asset_files[1].contains("webui-component-asset"));
    assert!(result.component_asset_files[1].contains("export default asset;"));
    assert!(!result.component_asset_files[1].contains("defineComponentAsset"));
    assert!(result.component_asset_files[2].starts_with("components/lazy-panel."));
    assert!(result.component_asset_files[2].ends_with(".webui.js"));
    assert!(result.component_asset_files[3].contains("export default asset;"));
    let metafile = result
        .metafile
        .as_deref()
        .expect("requested metafile must be returned");
    let parsed: serde_json::Value = serde_json::from_str(metafile).unwrap();
    assert!(parsed["inputs"].get("webui:component/lazy-panel").is_some());
    assert!(parsed["outputs"].get("lazy-panel.webui.js").is_some());
}

#[test]
fn test_build_legal_comments_none_strips_legal_css() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("index.html"), "<my-card>Hello</my-card>").unwrap();
    std::fs::write(
        dir.path().join("my-card.html"),
        r#"<template shadowrootmode="open"><div>content</div></template>"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("my-card.css"),
        "/*! @license MIT */ .card { color: red; }",
    )
    .unwrap();

    let options = JsBuildOptions {
        app_dir: dir.path().to_string_lossy().to_string(),
        entry: None,
        css: Some("link".to_string()),
        dom: None,
        css_bundle: None,
        plugin: None,
        components: None,
        component_asset_roots: None,
        metafile: None,
        css_file_name_template: None,
        asset_base: None,
        component_modules: None,
        modules_out: None,
        legal_comments: Some("none".to_string()),
        theme: None,
        projection_manifests: None,
        projection_manifest_objects: None,
    };

    let result = build(options).unwrap();
    assert_eq!(result.css_files[1], " .card { color: red; }");
}

#[test]
fn test_build_invalid_legal_comments() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("index.html"), "<h1>Hello</h1>").unwrap();

    let options = JsBuildOptions {
        app_dir: dir.path().to_string_lossy().to_string(),
        entry: None,
        css: None,
        dom: None,
        css_bundle: None,
        plugin: None,
        components: None,
        component_asset_roots: None,
        metafile: None,
        css_file_name_template: None,
        asset_base: None,
        component_modules: None,
        modules_out: None,
        legal_comments: Some("linked".to_string()),
        theme: None,
        projection_manifests: None,
        projection_manifest_objects: None,
    };

    let result = build(options);
    assert!(result.is_err());
}

#[test]
fn test_build_with_unwrapped_component_defaults_to_shadow() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("index.html"), "<my-card>Hi</my-card>").unwrap();
    std::fs::write(dir.path().join("my-card.html"), "<div>content</div>").unwrap();

    let options = JsBuildOptions {
        app_dir: dir.path().to_string_lossy().to_string(),
        entry: None,
        css: None,
        dom: None,
        css_bundle: None,
        plugin: None,
        components: None,
        component_asset_roots: None,
        metafile: None,
        css_file_name_template: None,
        asset_base: None,
        component_modules: None,
        modules_out: None,
        legal_comments: None,
        theme: None,
        projection_manifests: None,
        projection_manifest_objects: None,
    };

    let result = build(options).unwrap();
    let json = inspect(result.protocol).unwrap();
    assert!(json.contains("shadowrootmode"));
}

#[test]
fn test_build_with_light_dom_omits_generated_shadow_root() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("index.html"), "<my-card>Hi</my-card>").unwrap();
    std::fs::write(dir.path().join("my-card.html"), "<div>content</div>").unwrap();

    let options = JsBuildOptions {
        app_dir: dir.path().to_string_lossy().to_string(),
        entry: None,
        css: None,
        dom: Some("light".to_string()),
        css_bundle: None,
        plugin: None,
        components: None,
        component_asset_roots: None,
        metafile: None,
        css_file_name_template: None,
        asset_base: None,
        component_modules: None,
        modules_out: None,
        legal_comments: None,
        theme: None,
        projection_manifests: None,
        projection_manifest_objects: None,
    };

    let result = build(options).unwrap();
    let json = inspect(result.protocol).unwrap();
    assert!(!json.contains("shadowrootmode"));
}

#[test]
fn test_build_with_authored_shadow_dom_preserves_shadow_root_template() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("index.html"), "<my-card>Hi</my-card>").unwrap();
    std::fs::write(
        dir.path().join("my-card.html"),
        r#"<template shadowrootmode="open"><div><slot></slot></div></template>"#,
    )
    .unwrap();

    let options = JsBuildOptions {
        app_dir: dir.path().to_string_lossy().to_string(),
        entry: None,
        css: None,
        dom: None,
        css_bundle: None,
        plugin: None,
        components: None,
        component_asset_roots: None,
        metafile: None,
        css_file_name_template: None,
        asset_base: None,
        component_modules: None,
        modules_out: None,
        legal_comments: None,
        theme: None,
        projection_manifests: None,
        projection_manifest_objects: None,
    };

    let result = build(options).unwrap();
    let json = inspect(result.protocol).unwrap();
    assert!(
        json.contains("shadowrootmode"),
        "authored Shadow DOM wrapper should be preserved, got: {json}"
    );
}

#[test]
fn test_build_rejects_invalid_shadow_wrapper() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("index.html"), "<my-card></my-card>").unwrap();
    std::fs::write(
        dir.path().join("my-card.html"),
        r#"<template shadowrootmode="closed"><p>invalid</p></template>"#,
    )
    .unwrap();

    let options = JsBuildOptions {
        app_dir: dir.path().to_string_lossy().to_string(),
        entry: None,
        css: None,
        dom: None,
        css_bundle: None,
        plugin: None,
        components: None,
        component_asset_roots: None,
        metafile: None,
        css_file_name_template: None,
        asset_base: None,
        component_modules: None,
        modules_out: None,
        legal_comments: None,
        theme: None,
        projection_manifests: None,
        projection_manifest_objects: None,
    };

    let result = build(options);
    assert!(result.is_err());
}
