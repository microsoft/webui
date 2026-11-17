// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn requires_one_explicit_provenance_source_and_target_cli() -> Result<(), String> {
    for values in [
        vec![],
        vec![
            "--target",
            "aarch64-apple-darwin",
            "--cli",
            "webui",
            "--checker-dir",
            "package",
        ],
        vec![
            "--target",
            "aarch64-apple-darwin",
            "--cli",
            "webui",
            "--checker-package",
            "ts.tgz",
            "--acquire-checker",
        ],
        vec!["--target", "aarch64-apple-darwin", "--acquire-checker"],
        vec!["--target", "--cli", "webui", "--acquire-checker"],
        vec!["--unknown"],
        vec![
            "--target",
            "aarch64-apple-darwin",
            "--target",
            "other",
            "--cli",
            "webui",
            "--acquire-checker",
        ],
    ] {
        assert!(parse(&args(&values)).is_err(), "{values:?}");
    }
    let options = parse(&args(&[
        "--target",
        "x86_64-pc-windows-msvc",
        "--cli",
        "webui.exe",
        "--acquire-checker",
    ]))?;
    assert_eq!(options.target, "x86_64-pc-windows-msvc");
    assert!(options.package.is_none());
    let options = parse(&args(&[
        "--target",
        "aarch64-apple-darwin",
        "--cli",
        "webui",
        "--checker-package",
        "ts.tgz",
        "--checker-dir",
        "package",
    ]))?;
    assert_eq!(options.package, Some(PathBuf::from("ts.tgz")));
    assert_eq!(options.checker_dir, Some(PathBuf::from("package")));
    Ok(())
}

#[test]
fn help_is_available_without_artifacts_or_network() {
    assert_eq!(
        run(Path::new("does-not-exist"), &args(&["--help"])),
        ExitCode::SUCCESS
    );
}

#[test]
fn release_pipeline_yaml_remains_parseable() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for relative in [
        ".ado/pipelines/azure-pipelines-build.yml",
        ".ado/pipelines/azure-pipelines-cd.yml",
    ] {
        let bytes = std::fs::read(root.join(relative))?;
        let _: serde_yaml::Value = serde_yaml::from_slice(&bytes)?;
    }
    Ok(())
}
