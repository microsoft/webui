// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Compares building the desktop protocol from source on every launch
//! (`DesktopRuntime::from_source`) against loading a protocol that was
//! compiled once ahead of time into a bundle (`DesktopRuntime::from_bundle_config_and_manifest`).
//! The bundle is built once outside the measured loop, mirroring `webui-desktop build`
//! running in CI/packaging rather than at app startup.

use std::hint::black_box;
use std::path::{Path, PathBuf};

use criterion::{criterion_group, criterion_main, Criterion};
use tempfile::TempDir;
use webui_desktop::{
    build_desktop_bundle, BuildOptions, CssStrategy, DesktopBundleConfig, DesktopBundleOptions,
    DesktopRuntime, DesktopShellConfig, DesktopSourceConfig, DomStrategy, Plugin, WindowOptions,
    DEFAULT_CSS_FILE_NAME_TEMPLATE,
};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(4)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn build_options(app_dir: PathBuf) -> BuildOptions {
    BuildOptions {
        app_dir,
        entry: "index.html".to_string(),
        css: CssStrategy::Link,
        dom: DomStrategy::Shadow,
        plugin: Some(Plugin::WebUI),
        css_file_name_template: DEFAULT_CSS_FILE_NAME_TEMPLATE.to_string(),
        ..BuildOptions::default()
    }
}

/// The `dist` directory holds both the esbuild-generated JS chunks and the
/// Rust-generated CSS files under the same basenames as `webui build` also
/// emits. Bundling copies build-emitted CSS alongside the asset root, so
/// only the JS chunks are copied here to avoid a duplicate-name collision.
fn copy_client_js(dist: &Path, out: &Path) {
    std::fs::create_dir_all(out).unwrap_or_else(|error| panic!("cannot create {out:?}: {error}"));
    for entry in
        std::fs::read_dir(dist).unwrap_or_else(|error| panic!("cannot read {dist:?}: {error}"))
    {
        let entry = entry.unwrap_or_else(|error| panic!("cannot read dir entry: {error}"));
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("js") {
            let target = out.join(path.file_name().unwrap_or_default());
            std::fs::copy(&path, &target)
                .unwrap_or_else(|error| panic!("cannot copy {path:?}: {error}"));
        }
    }
}

fn desktop_startup(c: &mut Criterion) {
    let root = workspace_root();
    let app_root = root.join("examples/app/contact-book-manager");
    let app_dir = app_root.join("src");
    let state_path = app_root.join("data/state.json");
    let assets_dir =
        TempDir::new().unwrap_or_else(|error| panic!("cannot create tempdir: {error}"));
    copy_client_js(&app_root.join("dist"), assets_dir.path());
    let assets = assets_dir.path().to_path_buf();

    // Compile the protocol once ahead of time, outside the measured loop - this is the
    // "build time" side of the comparison, equivalent to `webui-desktop build` in CI.
    let bundle_dir =
        TempDir::new().unwrap_or_else(|error| panic!("cannot create tempdir: {error}"));
    let manifest = build_desktop_bundle(DesktopBundleOptions {
        build_options: build_options(app_dir.clone()),
        out_dir: bundle_dir.path().to_path_buf(),
        state_file: Some(state_path.clone()),
        asset_root: Some(assets.clone()),
        token_css: None,
        app_id: "com.microsoft.webui.contactbook.bench".to_string(),
        app_name: "Contact Book Manager".to_string(),
        version: "0.0.0".to_string(),
        publisher: "Microsoft".to_string(),
        window: WindowOptions::default(),
        icon_file: None,
        shell: DesktopShellConfig::default(),
        package_targets: Vec::new(),
    })
    .unwrap_or_else(|error| panic!("bundle build failed: {error}"));

    let mut group = c.benchmark_group("desktop_startup");

    group.bench_function("from_bundle_precompiled_protocol", |b| {
        b.iter(|| {
            let config = DesktopBundleConfig::new(bundle_dir.path().to_path_buf());
            let runtime = DesktopRuntime::from_bundle_config_and_manifest(config, manifest.clone())
                .unwrap_or_else(|error| panic!("from_bundle failed: {error}"));
            black_box(runtime);
        });
    });

    group.bench_function("from_source_compiled_at_launch", |b| {
        b.iter(|| {
            let mut config = DesktopSourceConfig::new(build_options(app_dir.clone()));
            config.state_file = Some(state_path.clone());
            config.asset_root = Some(assets.clone());
            let runtime = DesktopRuntime::from_source(config)
                .unwrap_or_else(|error| panic!("from_source failed: {error}"));
            black_box(runtime);
        });
    });

    group.finish();
}

criterion_group!(benches, desktop_startup);
criterion_main!(benches);
