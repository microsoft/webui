// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::Path;

use microsoft_webui_client_build::{build, ClientBuildOptions};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn options(root: &Path) -> ClientBuildOptions {
    ClientBuildOptions {
        project: root.to_owned(),
        entries: [("index".into(), "index.ts".into())].into(),
        out_dir: "dist".into(),
        tsconfig: Some("tsconfig.json".into()),
        ..Default::default()
    }
}

fn fixture(root: &Path) -> TestResult {
    fs::write(
        root.join("index.ts"),
        "export const message: string = 'safe';",
    )?;
    fs::write(root.join("tsconfig.json"), r#"{"extends":"./base.json"}"#)?;
    fs::write(
        root.join("base.json"),
        r#"{"compilerOptions":{"experimentalDecorators":true}}"#,
    )?;
    Ok(())
}

#[tokio::test]
async fn configuration_manifest_paths_and_portable_case_collisions_never_overwrite() -> TestResult {
    let temp = tempfile::tempdir()?;
    fixture(temp.path())?;
    let result = build(options(temp.path())).await?;
    let root = result.project();
    assert!(result.dependencies.contains(&root.join("tsconfig.json")));
    for path in [
        "tsconfig.json",
        "TSCONFIG.json",
        "base.json",
        "dist/INDEX.js",
    ] {
        assert!(result.write_outputs(Path::new(path)).is_err(), "{path}");
        assert_eq!(
            fs::read_to_string(root.join("tsconfig.json"))?,
            r#"{"extends":"./base.json"}"#
        );
        assert_eq!(
            fs::read_to_string(root.join("base.json"))?,
            r#"{"compilerOptions":{"experimentalDecorators":true}}"#
        );
        assert!(!root.join("dist").exists());
    }
    Ok(())
}

#[tokio::test]
async fn actual_resolved_package_metadata_is_a_protected_watch_dependency() -> TestResult {
    let temp = tempfile::tempdir()?;
    fixture(temp.path())?;
    let package = temp.path().join("node_modules/example");
    fs::create_dir_all(&package)?;
    let json = r#"{"name":"example","type":"module","exports":"./index.js"}"#;
    fs::write(package.join("package.json"), json)?;
    fs::write(package.join("index.js"), "export const value = 42;")?;
    fs::write(
        temp.path().join("index.ts"),
        "export { value } from 'example';",
    )?;
    let result = build(options(temp.path())).await?;
    let metadata = package.canonicalize()?.join("package.json");
    assert!(result.dependencies.contains(&metadata));
    assert!(result.write_outputs(&metadata).is_err());
    assert_eq!(fs::read_to_string(metadata)?, json);
    Ok(())
}

#[tokio::test]
async fn publication_uses_only_declared_staging_roots_and_preserves_unknown_contents() -> TestResult
{
    let temp = tempfile::tempdir()?;
    fixture(temp.path())?;
    let result = build(options(temp.path())).await?;
    let manifest = Path::new("proofs/current.json");
    let roots = result.publication_temporary_roots(manifest)?;
    assert_eq!(roots.len(), 2);
    for root in &roots {
        fs::create_dir_all(root)?;
        fs::write(root.join("unknown-user-file"), "keep")?;
    }
    result.write_outputs(manifest)?;
    for root in &roots {
        assert_eq!(fs::read_to_string(root.join("unknown-user-file"))?, "keep");
        assert_eq!(fs::read_dir(root)?.count(), 1);
    }
    Ok(())
}

#[tokio::test]
async fn inputs_in_owned_staging_roots_and_unowned_existing_outputs_are_rejected() -> TestResult {
    let temp = tempfile::tempdir()?;
    fixture(temp.path())?;
    fs::create_dir_all(temp.path().join("dist/.webui-bundle-staging"))?;
    fs::write(
        temp.path().join("dist/.webui-bundle-staging/authored.ts"),
        "export const value = 1;",
    )?;
    fs::write(
        temp.path().join("index.ts"),
        "export {value} from './dist/.webui-bundle-staging/authored.ts';",
    )?;
    let result = build(options(temp.path())).await?;
    assert!(result
        .write_outputs(Path::new("dist/projection.json"))
        .is_err());
    fs::write(temp.path().join("index.ts"), "export const value = 1;")?;
    fs::write(
        temp.path().join("dist/index.js"),
        "unrelated existing document",
    )?;
    let result = build(options(temp.path())).await?;
    assert!(result
        .write_outputs(Path::new("dist/projection.json"))
        .is_err());
    assert_eq!(
        fs::read_to_string(temp.path().join("dist/index.js"))?,
        "unrelated existing document"
    );
    Ok(())
}
