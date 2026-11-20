// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::Path;

use webui::browser_build::{build, ClientBuildOptions, Loader, ScriptSyntax};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn options(root: &Path) -> ClientBuildOptions {
    ClientBuildOptions {
        project: root.to_owned(),
        entries: [("index".into(), "index.ts".into())].into(),
        out_dir: "dist".into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn explicit_writer_publishes_exact_bytes_and_returns_the_written_proof() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?;
    fs::write(root.join("index.ts"), "export const message = 'hello';")?;
    let result = build(options(&root)).await?;
    assert!(!root.join("dist").exists());
    let path = root.join("dist/projection.json");
    let proof = result.write_outputs(&path)?;
    assert_eq!(fs::read_to_string(&path)?, serde_json::to_string(&proof)?);
    for (name, bytes) in &result.files {
        assert_eq!(fs::read(root.join(name))?, *bytes);
    }
    let sources = [webui::ProjectionManifestSource::Path(path.clone())];
    webui::prepare_projection_manifests(&sources)?;
    // Rebuild publication atomically replaces regular files and leaves no temp files.
    fs::write(root.join("dist/index.js"), "old generation")?;
    result.write_outputs(Path::new("dist/projection.json"))?;
    webui::prepare_projection_manifests(&sources)?;
    assert!(fs::read_dir(root.join("dist"))?.all(|entry| entry
        .ok()
        .is_some_and(|entry| !entry.file_name().to_string_lossy().starts_with(".tmp"))));
    Ok(())
}

#[tokio::test]
async fn identical_generations_preserve_outputs_but_stale_bytes_are_repaired() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    fs::write(root.join("index.ts"), "export const message = 'hello';")?;
    let result = build(options(root)).await?;
    let manifest = Path::new("dist/projection.json");
    let output = Path::new("dist/index.js");
    result.write_outputs(manifest)?;
    let old_time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
    for path in [output, manifest] {
        fs::File::options()
            .write(true)
            .open(root.join(path))?
            .set_times(fs::FileTimes::new().set_modified(old_time))?;
    }

    result.write_outputs(manifest)?;
    for path in [output, manifest] {
        assert_eq!(fs::metadata(root.join(path))?.modified()?, old_time);
    }

    let expected = &result.files[output];
    fs::write(root.join(output), vec![b' '; expected.len()])?;
    result.write_outputs(manifest)?;
    assert_eq!(&fs::read(root.join(output))?, expected);
    assert_ne!(fs::metadata(root.join(manifest))?.modified()?, old_time);
    Ok(())
}

#[tokio::test]
async fn publication_rejects_collisions_and_inputs_before_writing_any_output() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?;
    let source = "export const message = 'original input';";
    fs::write(root.join("index.ts"), source)?;
    let result = build(options(&root)).await?;
    for manifest in ["dist/index.js", "index.ts", "../escape.json"] {
        assert!(
            result.write_outputs(Path::new(manifest)).is_err(),
            "{manifest}"
        );
        assert!(!root.join("dist").exists());
        assert_eq!(fs::read_to_string(root.join("index.ts"))?, source);
    }
    let outside = tempfile::tempdir()?;
    assert!(result
        .write_outputs(&outside.path().join("proof.json"))
        .is_err());
    let mut config = options(&root);
    config.out_dir = "scripts".into();
    config.entry_names = "../index.ts".into();
    assert!(build(config).await.is_err());
    let mut config = options(&root);
    config.out_dir = "dist".into();
    fs::write(root.join("dist.ts"), "export const value = 2;")?;
    config.entries.insert("other".into(), "dist.ts".into());
    let mut result = build(config).await?;
    result
        .files
        .insert("index.ts".into(), b"overwritten".to_vec());
    assert!(result.write_outputs(Path::new("dist/proof.json")).is_err());
    assert!(!root.join("dist").exists());
    assert_eq!(fs::read_to_string(root.join("index.ts"))?, source);
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn publication_rejects_redirects_and_does_not_truncate_hardlinked_inputs() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?;
    let outside = tempfile::tempdir()?;
    let source = "export const original = 'source';";
    fs::write(root.join("index.ts"), source)?;
    let result = build(options(&root)).await?;
    std::os::unix::fs::symlink(outside.path(), root.join("dist"))?;
    assert!(result
        .write_outputs(Path::new("dist/projection.json"))
        .is_err());
    assert!(fs::read_dir(outside.path())?.next().is_none());
    fs::remove_file(root.join("dist"))?;
    fs::create_dir(root.join("dist"))?;
    fs::write(outside.path().join("target.js"), "outside")?;
    std::os::unix::fs::symlink(outside.path().join("target.js"), root.join("dist/index.js"))?;
    assert!(result
        .write_outputs(Path::new("dist/projection.json"))
        .is_err());
    assert_eq!(
        fs::read_to_string(outside.path().join("target.js"))?,
        "outside"
    );
    fs::remove_file(root.join("dist/index.js"))?;
    result.write_outputs(Path::new("dist/projection.json"))?;
    fs::remove_file(root.join("dist/index.js"))?;
    fs::hard_link(root.join("index.ts"), root.join("dist/index.js"))?;
    result.write_outputs(Path::new("dist/projection.json"))?;
    assert_eq!(fs::read_to_string(root.join("index.ts"))?, source);
    assert_ne!(fs::read_to_string(root.join("dist/index.js"))?, source);
    Ok(())
}

#[tokio::test]
async fn actual_ts_loader_grammar_survives_nonstandard_extensions() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?;
    let package = root.join("node_modules/@microsoft/webui-framework");
    fs::create_dir_all(&package)?;
    fs::write(
        package.join("package.json"),
        r#"{"name":"@microsoft/webui-framework","type":"module","exports":"./index.js"}"#,
    )?;
    fs::write(
        package.join("index.js"),
        "export const observable = () => {};",
    )?;
    fs::write(root.join("counter.custom"), "import { observable } from '@microsoft/webui-framework'; class Counter { @observable count: number = 0; } Counter.define('custom-counter');")?;
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"experimentalDecorators":true}}"#,
    )?;
    let mut config = options(&root);
    config
        .entries
        .insert("index".into(), "counter.custom".into());
    config.loaders.insert(".custom".into(), Loader::Ts);
    let result = build(config).await?;
    let id = result.graph.entries.first().ok_or("missing entry module")?;
    assert_eq!(Path::new(id).canonicalize()?, root.join("counter.custom"));
    assert_eq!(result.graph.modules[id].syntax, Some(ScriptSyntax::Ts));
    let proof = result.projection_manifest(Path::new("dist/projection.json"))?;
    assert_eq!(proof.components["custom-counter"].hydration_keys, ["count"]);
    Ok(())
}
