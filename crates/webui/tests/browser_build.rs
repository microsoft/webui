// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::Path;

use webui::browser_build::{build, ClientBuildOptions, SourceMap};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn fixture() -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    fs::create_dir(root.join("framework"))?;
    fs::write(
        root.join("framework/package.json"),
        format!(
            r#"{{"name":"@microsoft/webui-framework","version":"{}","exports":{{".":{{"import":{{"default":"./index.js"}}}}}}}}"#,
            env!("CARGO_PKG_VERSION")
        ),
    )?;
    fs::write(root.join("framework/index.js"),
        "export function observable(target, name) { Object.defineProperty(target, name, { set(v) { this._value = v; }, get() { return this._value; } }); }")?;
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"experimentalDecorators":true,"useDefineForClassFields":false}}"#,
    )?;
    Ok(temp)
}

fn options(root: &Path) -> ClientBuildOptions {
    ClientBuildOptions {
        project: root.to_owned(),
        entries: [("index".into(), "index.ts".into())].into(),
        out_dir: ".webui/stage".into(),
        framework: Some("framework".into()),
        development: false,
        sourcemap: SourceMap::File,
        ..Default::default()
    }
}

#[tokio::test]
async fn retains_original_typescript_and_exact_resolutions_without_publishing() -> TestResult {
    let temp = fixture()?;
    let root = temp.path().canonicalize()?;
    let source = "import { observable } from '@microsoft/webui-framework'; export class Counter { @observable count = 0; increment(): void { this.count += 1; } }";
    fs::write(root.join("index.ts"), source)?;
    let result = build(options(&root)).await?;
    assert!(
        !root.join(".webui").exists(),
        "assets must remain uncommitted until proof finalization"
    );
    let entry = root.join("index.ts").to_string_lossy().replace('\\', "/");
    let module = result
        .graph
        .modules
        .get(&entry)
        .ok_or("missing original module")?;
    assert_eq!(module.source.as_deref(), Some(source));
    let import = module.imports.first().ok_or("missing import")?;
    assert_eq!(import.specifier, "@microsoft/webui-framework");
    assert!(import.resolved.ends_with("/framework/index.js"));
    assert!(!import.external);
    assert!(result
        .membership
        .values()
        .any(|inputs| inputs.contains(&entry)));
    let code = result
        .files
        .get(Path::new(".webui/stage/index.js"))
        .ok_or("missing JavaScript")?;
    assert!(code.is_ascii());
    assert!(!String::from_utf8_lossy(code).contains("@observable"));
    assert!(result
        .files
        .keys()
        .any(|file| file.extension().is_some_and(|ext| ext == "map")));
    Ok(())
}

#[tokio::test]
async fn production_strips_dev_import_outputs_and_preserves_dynamic_splits() -> TestResult {
    let temp = fixture()?;
    let root = temp.path();
    fs::write(root.join("index.ts"),
        "if (__WEBUI_DEV__) { import('./diagnostic.js'); } export const load = () => import('./feature.js');")?;
    fs::write(
        root.join("diagnostic.js"),
        "console.error('PRIVATE_DEV_DIAGNOSTIC');",
    )?;
    fs::write(root.join("feature.js"), "export const message = 'Feature';")?;
    let result = build(options(root)).await?;
    for (name, bytes) in &result.files {
        if name.extension().is_some_and(|ext| ext == "js") {
            assert!(!String::from_utf8_lossy(bytes).contains("PRIVATE_DEV_DIAGNOSTIC"));
        }
    }
    assert!(result
        .files
        .keys()
        .any(|name| name.to_string_lossy().contains("feature")));
    assert!(!result
        .files
        .keys()
        .any(|name| name.to_string_lossy().contains("diagnostic")));
    Ok(())
}

#[tokio::test]
async fn rejects_escaping_sources_outputs_and_mismatched_runtime_resources() -> TestResult {
    let temp = fixture()?;
    let outside = tempfile::tempdir()?;
    fs::write(outside.path().join("outside.js"), "export const value = 1;")?;
    fs::write(temp.path().join("index.ts"), "export const value = 1;")?;
    let mut escaping = options(temp.path());
    escaping.out_dir = "../outside".into();
    assert!(build(escaping).await.is_err());
    let mut escaping = options(temp.path());
    escaping
        .entries
        .insert("index".into(), outside.path().join("outside.js"));
    assert!(build(escaping).await.is_err());
    fs::write(
        temp.path().join("framework/package.json"),
        r#"{"name":"@microsoft/webui-framework","version":"0.0.0","exports":{}}"#,
    )?;
    assert!(build(options(temp.path())).await.is_err());
    Ok(())
}

#[tokio::test]
async fn produces_a_real_final_byte_proof_and_rejects_stale_output() -> TestResult {
    let temp = fixture()?;
    let root = temp.path().canonicalize()?;
    fs::write(root.join("index.ts"),
        "import { observable } from '@microsoft/webui-framework'; class Counter { @observable count = 0; } Counter.define('app-counter');")?;
    let result = build(options(&root)).await?;
    let path = Path::new(".webui/stage/projection.json");
    let manifest = result.projection_manifest(path)?;
    let component = manifest
        .components
        .get("app-counter")
        .ok_or("missing proven component")?;
    assert_eq!(component.hydration_keys, ["count"]);
    for (name, bytes) in &result.files {
        let output = root.join(name);
        fs::create_dir_all(output.parent().ok_or("output parent")?)?;
        fs::write(output, bytes)?;
    }

    let json = serde_json::to_string(&manifest)?;
    let sources = [webui::ProjectionManifestSource::Inline {
        manifest_path: root.join(path),
        json,
    }];
    webui::prepare_projection_manifests(&sources)?;
    fs::write(root.join(".webui/stage/index.js"), "stale output")?;
    let Err(error) = webui::prepare_projection_manifests(&sources) else {
        return Err("stale JavaScript must invalidate the final-byte proof".into());
    };
    assert!(error.to_string().contains("PROJ-M004"), "{error}");
    Ok(())
}

#[tokio::test]
async fn bounds_physical_source_reads_at_the_exact_module_limit() -> TestResult {
    use std::io::Write;
    let temp = fixture()?;
    let path = temp.path().join("index.ts");
    let mut file = fs::File::create(&path)?;
    let prefix = b"export const value = 1;\n";
    file.write_all(prefix)?;
    let whitespace = [b' '; 4096];
    let mut remaining = 16 * 1024 * 1024 - prefix.len();
    while remaining > 0 {
        let count = remaining.min(whitespace.len());
        file.write_all(&whitespace[..count])?;
        remaining -= count;
    }
    drop(file);
    let mut configuration = options(temp.path());
    configuration.sourcemap = SourceMap::None;
    build(configuration).await?;
    fs::OpenOptions::new()
        .append(true)
        .open(&path)?
        .write_all(b" ")?;
    let Err(error) = build(options(temp.path())).await else {
        return Err("a module one byte beyond the limit was accepted".into());
    };
    assert!(error.to_string().contains("16 MiB"), "{error}");
    Ok(())
}
