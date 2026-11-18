// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use webui::browser_build::{build, ClientBuildOptions, Format, Loader, SourceMap};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn options(root: &Path) -> ClientBuildOptions {
    ClientBuildOptions {
        project: root.to_owned(),
        entries: [("assets/page-1".into(), "index.ts".into())].into(),
        out_dir: "dist".into(),
        ..Default::default()
    }
}

#[test]
fn serde_configuration_requires_boundaries_and_rejects_unknown_policy() -> TestResult {
    let options: ClientBuildOptions = serde_json::from_str(
        r#"{"project":"/project","entries":{"assets/page-1":"src/index.ts"},"outDir":"dist"}"#,
    )?;
    assert!(options.minify);
    assert!(!options.development);
    assert_eq!(options.target, "es2022");
    assert_eq!(options.entry_names, "[name].js");
    assert_eq!(options.sourcemap, SourceMap::None);
    for json in [
        r#"{"project":"/project","outDir":"dist"}"#,
        r#"{"project":"/project","entries":{},"outDir":"dist","plugins":[]}"#,
        r#"{"project":"/project","entries":{},"outDir":"dist","sourcemap":true}"#,
        r#"{"project":"/project","entries":{},"outDir":"dist","loaders":{".x":"unknown"}}"#,
    ] {
        assert!(
            serde_json::from_str::<ClientBuildOptions>(json).is_err(),
            "{json}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn nested_multi_entry_closures_use_emitted_paths_not_the_entry_directory() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    fs::write(
        root.join("index.ts"),
        "export { shared } from './shared.ts'; export const lazy = () => import('./feature.ts');",
    )?;
    fs::write(
        root.join("other.ts"),
        "export { shared } from './shared.ts';",
    )?;
    fs::write(
        root.join("shared.ts"),
        "export const shared = globalThis.SharedValue;",
    )?;
    fs::write(
        root.join("feature.ts"),
        "export const feature = 'deferred';",
    )?;
    let mut config = options(root);
    config
        .entries
        .insert("deep/other/page-2".into(), "other.ts".into());
    config.chunk_names = "chunks/[name]-[hash].js".into();
    config.public_path = Some("/site/".into());
    config.sourcemap = SourceMap::File;
    let result = build(config).await?;
    let manifest = result.projection_manifest(Path::new("dist/projection.json"))?;
    assert_eq!(result.entries.len(), 2);
    assert_eq!(result.project(), root.canonicalize()?);
    for entry in &result.entries {
        let details = result
            .metadata
            .outputs
            .get(entry.to_str().ok_or("UTF-8")?)
            .ok_or("metadata")?;
        assert!(details
            .entry_point
            .as_ref()
            .is_some_and(|id| id.starts_with('/')));
        let closure = manifest
            .entry_closures
            .get(entry.to_str().ok_or("UTF-8")?)
            .ok_or("closure")?;
        assert!(!closure.is_empty(), "{closure:?}");
        for path in closure {
            assert!(result.files.contains_key(Path::new(path)));
            assert!(
                !path.contains("feature"),
                "dynamic edges must never become preloads"
            );
        }
        let code = String::from_utf8_lossy(&result.files[entry]);
        assert!(code.contains("/site/chunks/"), "{code}");
    }
    let json = serde_json::to_value(&result.metadata)?;
    assert!(json.to_string().contains("renderedBytesBeforeMinify"));
    assert!(!json.to_string().contains("bytesInOutput"));
    assert!(!root.join("dist").exists());
    Ok(())
}

#[tokio::test]
async fn native_virtual_alias_text_and_css_attributes_capture_authored_bytes() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?;
    fs::write(root.join("index.ts"), "import { appearance } from 'press:appearance'; import markup from './template.html'; import text from './opaque.ts'; import sheet from './theme.css' with { type: 'css' }; export { appearance, markup, text, sheet };")?;
    fs::write(root.join("template.html"), "<h1>café 😀</h1>")?;
    fs::write(
        root.join("opaque.ts"),
        "this is not JavaScript; define('invalid-component')",
    )?;
    fs::write(root.join("theme.css"), ":host { color: red; }")?;
    let mut config = options(&root);
    let virtual_id = "\0press:appearance.mts";
    config.virtual_modules.insert(
        virtual_id.into(),
        "export const appearance: string = 'stable';".into(),
    );
    config
        .aliases
        .insert("press:appearance$".into(), virtual_id.into());
    config.loaders = [(".html".into(), Loader::Text), (".ts".into(), Loader::Text)].into();
    // Keep the entry a real TypeScript input while proving a .ts text override.
    fs::rename(root.join("index.ts"), root.join("index.mts"))?;
    config
        .entries
        .insert("assets/page-1".into(), "index.mts".into());
    let result = build(config).await?;
    let opaque = root.join("opaque.ts").to_string_lossy().into_owned();
    let markup_id = root.join("template.html").to_string_lossy().into_owned();
    let markup_source = "<h1>café 😀</h1>";
    assert_eq!(
        result.metadata.inputs[&markup_id].bytes,
        markup_source.len()
    );
    assert_ne!(
        result.metadata.inputs[&markup_id].bytes,
        markup_source.encode_utf16().count()
    );
    assert!(result.graph.modules[&opaque].opaque);
    assert_eq!(
        result.metadata.inputs[&opaque].bytes,
        fs::read(root.join("opaque.ts"))?.len()
    );
    let virtual_module = result
        .graph
        .modules
        .get(virtual_id)
        .ok_or("stable virtual ID not retained")?;
    assert!(virtual_module
        .source
        .as_deref()
        .is_some_and(|source| source.contains(": string")));
    assert!(result.graph.modules.values().any(|module| module
        .imports
        .iter()
        .any(|edge| edge.specifier == "press:appearance" && edge.resolved == virtual_id)));
    let code = result
        .files
        .get(Path::new("dist/assets/page-1.js"))
        .ok_or("output")?;
    assert!(code.is_ascii());
    assert!(String::from_utf8_lossy(code).contains("CSSStyleSheet"));
    let report = webui_projection::analyze_with_report(&result.graph);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert!(!report.parsed_modules.contains(&opaque));
    result.projection_manifest(Path::new("dist/projection.json"))?;
    Ok(())
}

#[tokio::test]
async fn opaque_binary_assets_are_bounded_and_proven_without_utf8_coercion() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?;
    fs::write(
        root.join("index.ts"),
        "import file from './pixel.bin'; import data from './inline.dat'; export { file, data };",
    )?;
    let bytes = [0, 159, 146, 150, 255];
    fs::write(root.join("pixel.bin"), bytes)?;
    fs::write(root.join("inline.dat"), bytes)?;
    let mut config = options(&root);
    config.loaders = [
        (".bin".into(), Loader::File),
        (".dat".into(), Loader::DataUrl),
    ]
    .into();
    config.public_path = Some("https://cdn.example.test/static".into());
    let result = build(config).await?;
    let source = root.join("pixel.bin").to_string_lossy().into_owned();
    assert!(result.graph.modules[&source].opaque);
    assert!(result.graph.modules[&source].source.is_none());
    assert_eq!(result.metadata.inputs[&source].bytes, bytes.len());
    assert!(result.files.values().any(|value| value == &bytes));
    let code = String::from_utf8_lossy(&result.files[Path::new("dist/assets/page-1.js")]);
    assert!(code.contains("https://cdn.example.test/static/"), "{code}");
    assert!(
        code.contains("data:application/octet-stream;base64,"),
        "{code}"
    );
    // Manifest construction uses captured bytes, not a second filesystem sample.
    let before = result.projection_manifest(Path::new("dist/projection.json"))?;
    fs::write(root.join("pixel.bin"), "changed")?;
    fs::remove_file(root.join("inline.dat"))?;
    let after = result.projection_manifest(Path::new("dist/projection.json"))?;
    assert_eq!(before.build_id, after.build_id);
    assert_eq!(before.inputs["pixel.bin"], result.input_hashes[&source]);
    Ok(())
}

#[tokio::test]
async fn native_package_resolution_has_authoritative_identity_without_a_framework_pin() -> TestResult
{
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?;
    let package = root.join("node_modules/@microsoft/webui-framework");
    fs::create_dir_all(package.join("dist"))?;
    fs::write(
        package.join("package.json"),
        r#"{"name":"@microsoft/webui-framework","version":"3.1.2","type":"module","exports":{".":"./dist/index.js"}}"#,
    )?;
    fs::write(
        package.join("dist/index.js"),
        "export const observable = () => {};",
    )?;
    fs::write(root.join("index.ts"), "import { observable } from 'framework-alias'; class Counter { @observable count = 0; } Counter.define('app-counter');")?;
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"experimentalDecorators":true,"useDefineForClassFields":false}}"#,
    )?;
    let mut config = options(&root);
    config.aliases.insert(
        "framework-alias".into(),
        "@microsoft/webui-framework".into(),
    );
    config.tsconfig = Some("tsconfig.json".into());
    let result = build(config).await?;
    let source = root.join("index.ts").to_string_lossy().into_owned();
    let edge = result.graph.modules[&source]
        .imports
        .first()
        .ok_or("package edge")?;
    assert_eq!(
        edge.package_name.as_deref(),
        Some("@microsoft/webui-framework")
    );
    let manifest = result.projection_manifest(Path::new("dist/projection.json"))?;
    assert_eq!(manifest.components["app-counter"].hydration_keys, ["count"]);
    Ok(())
}

#[tokio::test]
async fn iife_inlines_dynamic_modules_and_preserves_legal_comments() -> TestResult {
    let temp = tempfile::tempdir()?;
    fs::write(temp.path().join("index.ts"), "/*! Copyright retained license */ export const load = () => import('./feature.ts'); globalThis.load = load; export const label = '你好'; globalThis.label = label;")?;
    fs::write(temp.path().join("feature.ts"), "export const value = 2;")?;
    let mut config = options(temp.path());
    config.format = Format::Iife;
    config.sourcemap = SourceMap::Inline;
    let result = build(config).await?;
    assert_eq!(result.files.len(), 1);
    let code = &result.files[Path::new("dist/assets/page-1.js")];
    assert!(code.is_ascii());
    let code = String::from_utf8_lossy(code);
    assert!(!code.contains("import("), "{code}");
    assert!(code.contains("Copyright retained license"), "{code}");
    assert!(code.contains("sourceMappingURL=data:"), "{code}");
    result.projection_manifest(Path::new("dist/projection.json"))?;
    Ok(())
}

#[tokio::test]
async fn output_external_edges_remain_external_and_hidden_maps_stay_hidden() -> TestResult {
    let temp = tempfile::tempdir()?;
    fs::write(temp.path().join("index.ts"), "export { value } from 'external-package'; export const load = () => import('external-dynamic');")?;
    let mut config = options(temp.path());
    config.external = vec!["external-package".into(), "external-dynamic".into()];
    config.sourcemap = SourceMap::Hidden;
    let result = build(config).await?;
    let metadata = &result.metadata.outputs["dist/assets/page-1.js"];
    assert_eq!(metadata.imports.len(), 2);
    assert!(metadata.imports.iter().all(|edge| edge.external));
    assert!(result.imports[Path::new("dist/assets/page-1.js")].is_empty());
    assert!(result
        .files
        .keys()
        .any(|path| path.extension().is_some_and(|ext| ext == "map")));
    assert!(
        !String::from_utf8_lossy(&result.files[Path::new("dist/assets/page-1.js")])
            .contains("sourceMappingURL=")
    );
    let manifest = result.projection_manifest(Path::new("dist/projection.json"))?;
    assert!(manifest.entry_closures["dist/assets/page-1.js"].is_empty());
    Ok(())
}

#[tokio::test]
async fn external_ids_matching_output_names_are_not_fabricated_internal_edges() -> TestResult {
    let temp = tempfile::tempdir()?;
    fs::write(
        temp.path().join("index.ts"),
        "export { remote } from 'feature.js'; export const lazy = () => import('./feature.ts');",
    )?;
    fs::write(temp.path().join("feature.ts"), "export const local = 1;")?;
    let mut config = options(temp.path());
    config.chunk_names = "[name].js".into();
    config.external.push("feature.js".into());
    config.public_path = Some("/static".into());
    let result = build(config).await?;
    let imports = &result.metadata.outputs["dist/assets/page-1.js"].imports;
    assert!(imports
        .iter()
        .any(|edge| edge.external && edge.path == "feature.js"));
    assert!(imports
        .iter()
        .any(|edge| !edge.external && edge.path == "dist/feature.js"));
    let manifest = result.projection_manifest(Path::new("dist/projection.json"))?;
    assert!(manifest.entry_closures["dist/assets/page-1.js"].is_empty());
    Ok(())
}

#[tokio::test]
async fn invalid_portable_paths_and_unsupported_css_fail_without_publishing() -> TestResult {
    let temp = tempfile::tempdir()?;
    fs::write(temp.path().join("index.ts"), "export const value = 1;")?;
    for name in ["../escape", "C:\\escape", "nested\\windows", "/absolute"] {
        let mut config = options(temp.path());
        config.entries = BTreeMap::from([(name.into(), "index.ts".into())]);
        assert!(build(config).await.is_err(), "{name}");
    }
    let mut config = options(temp.path());
    config.loaders.insert(".css".into(), Loader::Css);
    assert!(build(config).await.is_err());
    assert!(!temp.path().join("dist").exists());
    Ok(())
}

#[tokio::test]
async fn proof_ordering_is_case_sensitive_utf8_and_source_mutation_is_rejected() -> TestResult {
    let temp = tempfile::tempdir()?;
    let names = ["Z.ts", "a.ts", "é.ts", "\u{e000}.ts", "😀.ts"];
    let mut entry = String::new();
    for (index, name) in names.iter().enumerate() {
        fs::write(
            temp.path().join(name),
            format!("globalThis.value{index} = '{name}';"),
        )?;
        entry.push_str(&format!("import './{name}';"));
    }
    fs::write(temp.path().join("index.ts"), &entry)?;
    let mut result = build(options(temp.path())).await?;
    let manifest = result.projection_manifest(Path::new("dist/projection.json"))?;
    let inputs: Vec<_> = manifest
        .inputs
        .keys()
        .filter(|id| !id.starts_with("virtual:"))
        .map(String::as_str)
        .collect();
    assert_eq!(
        inputs,
        ["Z.ts", "a.ts", "index.ts", "é.ts", "\u{e000}.ts", "😀.ts"]
    );
    assert!(manifest.inputs.keys().all(|id| !id.contains('\\')));
    assert!(result.files[Path::new("dist/assets/page-1.js")].is_ascii());
    let module_id = temp
        .path()
        .canonicalize()?
        .join("index.ts")
        .to_string_lossy()
        .into_owned();
    result
        .graph
        .modules
        .get_mut(&module_id)
        .ok_or("entry")?
        .source = Some("modified".into());
    assert!(result
        .projection_manifest(Path::new("dist/projection.json"))
        .is_err());
    Ok(())
}

#[tokio::test]
async fn asset_limits_and_filesystem_boundary_are_enforced_before_native_emission() -> TestResult {
    let temp = tempfile::tempdir()?;
    fs::write(
        temp.path().join("index.ts"),
        "import data from './large.bin'; export { data };",
    )?;
    fs::File::create(temp.path().join("large.bin"))?.set_len(16 * 1024 * 1024 + 1)?;
    let mut config = options(temp.path());
    config.loaders.insert(".bin".into(), Loader::File);
    let Err(error) = build(config).await else {
        return Err("oversized opaque asset was accepted".into());
    };
    assert!(error.to_string().contains("16 MiB"), "{error}");
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir()?;
        fs::write(outside.path().join("secret.bin"), [255, 0])?;
        std::os::unix::fs::symlink(
            outside.path().join("secret.bin"),
            temp.path().join("escape.bin"),
        )?;
        fs::write(
            temp.path().join("index.ts"),
            "import data from './escape.bin'; export { data };",
        )?;
        let mut config = options(temp.path());
        config.loaders.insert(".bin".into(), Loader::File);
        assert!(build(config).await.is_err());
    }
    assert!(!temp.path().join("dist").exists());
    Ok(())
}
