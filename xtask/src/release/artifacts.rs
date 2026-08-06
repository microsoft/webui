// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::required_env;
use crate::publish;
use crate::util::workspace_root;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const ARTIFACT_MAPPINGS: &[(&str, &str)] = &[
    ("unsigned_npm_packages", "publish_artifacts_npm"),
    ("unsigned_crate_packages", "publish_artifacts_crates"),
    ("nuget_signing_input", "publish_artifacts_nuget"),
    ("standalone_release_assets", "publish_artifacts_standalone"),
];

pub(super) fn package() -> Result<(), String> {
    let output_root = PathBuf::from(required_env("PACKAGE_OUTPUT_ROOT")?);
    let stage_args = [
        "--pack-only".to_string(),
        "--profile".to_string(),
        "release".to_string(),
    ];
    if publish::run_stage(&stage_args) != ExitCode::SUCCESS {
        return Err("release package assembly failed".to_string());
    }

    let publish_root = workspace_root()?.join("publish");
    let npm = collect_extension(&publish_root.join("npm"), "tgz")?;
    let crates = collect_extension(&publish_root.join("crates"), "crate")?;
    let nuget = collect_extension(&publish_root.join("nuget"), "nupkg")?;
    let symbols = collect_extension(&publish_root.join("nuget"), "snupkg")?;
    let standalone = collect_files(&publish_root.join("standalone"))?;

    require_non_empty(&npm, "npm")?;
    require_non_empty(&crates, "crate")?;
    require_non_empty(&nuget, "NuGet package")?;
    require_non_empty(&symbols, "NuGet symbol package")?;
    if standalone.len() != 20 {
        return Err(format!(
            "expected 20 standalone release assets, found {}",
            standalone.len()
        ));
    }

    export_files(&output_root.join("publish_artifacts_npm"), &npm)?;
    export_files(&output_root.join("publish_artifacts_crates"), &crates)?;
    let mut all_nuget = Vec::with_capacity(nuget.len() + symbols.len());
    all_nuget.extend(nuget);
    all_nuget.extend(symbols);
    export_files(&output_root.join("publish_artifacts_nuget"), &all_nuget)?;
    export_files(
        &output_root.join("publish_artifacts_standalone"),
        &standalone,
    )
}

pub(super) fn stage_from_env() -> Result<(), String> {
    let source_root = PathBuf::from(required_env("PIPELINE_WORKSPACE")?).join("releaseBuild");
    let destination_root = PathBuf::from(required_env("BUILD_SOURCES_DIRECTORY")?);
    stage_artifacts(&source_root, &destination_root)?;
    println!("Staged all release package sets and standalone assets.");
    Ok(())
}

fn stage_artifacts(source_root: &Path, destination_root: &Path) -> Result<(), String> {
    for (artifact, destination) in ARTIFACT_MAPPINGS {
        let source = source_root.join(artifact);
        if !source.is_dir() {
            return Err(format!(
                "downloaded build artifact directory is missing: {}. Select a Web UI - CD Build run that assembled all required artifacts",
                source.display()
            ));
        }
        if collect_files(&source)?.is_empty() {
            return Err(format!(
                "downloaded build artifact directory is empty: {}. Select a successful Web UI - CD Build run with assembled release artifacts",
                source.display()
            ));
        }

        let destination = destination_root.join(destination);
        remove_dir_if_exists(&destination)?;
        copy_tree(&source, &destination)?;
    }
    Ok(())
}

fn collect_extension(dir: &Path, extension: &str) -> Result<Vec<PathBuf>, String> {
    let files = collect_files(dir)?;
    Ok(files
        .into_iter()
        .filter(|path| path.extension() == Some(OsStr::new(extension)))
        .collect())
}

fn collect_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries =
        fs::read_dir(dir).map_err(|error| format!("failed to read {}: {error}", dir.display()))?;
    let mut files = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("failed to read {} entry: {error}", dir.display()))?;
        if entry.path().is_file() {
            files.push(entry.path());
        }
    }
    files.sort();
    Ok(files)
}

fn require_non_empty(files: &[PathBuf], kind: &str) -> Result<(), String> {
    if files.is_empty() {
        Err(format!("no {kind} artifacts were rebuilt"))
    } else {
        Ok(())
    }
}

fn export_files(destination: &Path, files: &[PathBuf]) -> Result<(), String> {
    remove_dir_if_exists(destination)?;
    fs::create_dir_all(destination)
        .map_err(|error| format!("failed to create {}: {error}", destination.display()))?;
    for source in files {
        let Some(name) = source.file_name() else {
            return Err(format!("artifact has no file name: {}", source.display()));
        };
        let output = destination.join(name);
        fs::copy(source, &output).map_err(|error| {
            format!(
                "failed to copy {} to {}: {error}",
                source.display(),
                output.display()
            )
        })?;
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    let mut stack = Vec::with_capacity(4);
    stack.push((source.to_path_buf(), destination.to_path_buf()));
    while let Some((current_source, current_destination)) = stack.pop() {
        fs::create_dir_all(&current_destination).map_err(|error| {
            format!(
                "failed to create {}: {error}",
                current_destination.display()
            )
        })?;
        let entries = fs::read_dir(&current_source)
            .map_err(|error| format!("failed to read {}: {error}", current_source.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!("failed to read {} entry: {error}", current_source.display())
            })?;
            let source_path = entry.path();
            let destination_path = current_destination.join(entry.file_name());
            if source_path.is_dir() {
                stack.push((source_path, destination_path));
            } else if source_path.is_file() {
                fs::copy(&source_path, &destination_path).map_err(|error| {
                    format!(
                        "failed to copy {} to {}: {error}",
                        source_path.display(),
                        destination_path.display()
                    )
                })?;
            }
        }
    }
    Ok(())
}

fn remove_dir_if_exists(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_dir_all(path)
            .map_err(|error| format!("failed to clean {}: {error}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_artifacts_preserves_artifact_contents() {
        let source = tempfile::TempDir::new().expect("source should be created");
        let destination = tempfile::TempDir::new().expect("destination should be created");
        for (artifact, _) in ARTIFACT_MAPPINGS {
            let directory = source.path().join(artifact);
            fs::create_dir_all(&directory).expect("artifact directory should be created");
            fs::write(directory.join("asset"), artifact).expect("artifact should be written");
        }

        stage_artifacts(source.path(), destination.path()).expect("artifacts should be staged");

        for (artifact, output) in ARTIFACT_MAPPINGS {
            let value = fs::read_to_string(destination.path().join(output).join("asset"))
                .expect("staged artifact should be readable");
            assert_eq!(value, *artifact);
        }
    }

    #[test]
    fn stage_artifacts_rejects_empty_input() {
        let source = tempfile::TempDir::new().expect("source should be created");
        let destination = tempfile::TempDir::new().expect("destination should be created");
        for (artifact, _) in ARTIFACT_MAPPINGS {
            fs::create_dir_all(source.path().join(artifact))
                .expect("artifact directory should be created");
        }

        let error = stage_artifacts(source.path(), destination.path())
            .expect_err("empty input should fail");

        assert!(error.contains("is empty"));
    }
}
