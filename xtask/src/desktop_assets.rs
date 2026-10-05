// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::util::{run_command_quiet, workspace_root};

pub(crate) const ASSET_NAMES: [&str; 6] = [
    "native-bootstrap.js",
    "desktop-runtime.js",
    "local-native-bootstrap.js",
    "local-desktop-runtime.js",
    "linux-local-entry.js",
    "linux-local-mediator.js",
];

const GENERATED_DIRECTORY: &str = "target/webui-desktop-assets/ipc";
const PACKAGE_DIRECTORY: &str = "crates/webui-desktop/assets/ipc";

pub(crate) fn run() -> ExitCode {
    match generate() {
        Ok(path) => {
            eprintln!(
                "  {} Desktop browser assets staged in {}",
                console::style("✔").green(),
                console::style(path.display()).bold(),
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!(
                "  {} Desktop browser asset generation failed: {error}",
                console::style("✘").red().bold(),
            );
            ExitCode::FAILURE
        }
    }
}

pub(crate) fn generate() -> Result<PathBuf, String> {
    let root = workspace_root()?;
    run_command_quiet(
        "pnpm",
        &["--filter", "@microsoft/webui-desktop", "build"],
        None,
    )
    .map_err(|error| format!("desktop TypeScript build failed: {error}"))?;
    stage_built_assets(&root)
}

pub(crate) fn stage_built_assets(root: &Path) -> Result<PathBuf, String> {
    let destination = root.join(GENERATED_DIRECTORY);
    let destination_arg = path_argument(&destination)?;
    run_command_quiet(
        "node",
        &[
            "packages/webui-desktop/scripts/stage-rust-assets.mjs",
            "--write",
            destination_arg,
        ],
        Some(root),
    )
    .map_err(|error| format!("desktop Rust asset staging failed: {error}"))?;
    validate_assets(&destination)?;
    Ok(destination)
}

pub(crate) fn package_check() -> Result<(), String> {
    let root = workspace_root()?;
    generate()?;
    with_packaged_assets(&root, || {
        verify_package_file_list()?;
        run_command_quiet(
            "cargo",
            &[
                "package",
                "-p",
                "microsoft-webui*",
                "--no-verify",
                "--locked",
                "--allow-dirty",
            ],
            None,
        )
        .map_err(|error| format!("cargo package failed: {error}"))?;
        verify_packaged_consumer(&root)
    })
}

pub(crate) fn with_packaged_assets<T>(
    root: &Path,
    action: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let generated = root.join(GENERATED_DIRECTORY);
    validate_assets(&generated)?;
    let packaged = root.join(PACKAGE_DIRECTORY);
    with_staged_assets(&generated, &packaged, action)
}

fn with_staged_assets<T>(
    generated: &Path,
    packaged: &Path,
    action: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    if let Err(stage_error) = stage_package_directory(generated, packaged) {
        return match remove_package_directory(packaged) {
            Ok(()) => Err(stage_error),
            Err(cleanup_error) => Err(format!(
                "{stage_error}; cleanup also failed: {cleanup_error}"
            )),
        };
    }

    let result = action();
    let cleanup = remove_package_directory(packaged);
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(action_error), Err(cleanup_error)) => Err(format!(
            "{action_error}; cleanup also failed: {cleanup_error}"
        )),
    }
}

fn verify_package_file_list() -> Result<(), String> {
    let output = crate::util::build_command(
        "cargo",
        &[
            "package",
            "-p",
            "microsoft-webui-desktop",
            "--allow-dirty",
            "--no-verify",
            "--list",
        ],
    )
    .output()
    .map_err(|error| format!("failed to inspect desktop crate package: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "desktop crate package inspection failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let files = String::from_utf8(output.stdout)
        .map_err(|error| format!("desktop crate package list was not UTF-8: {error}"))?;
    for name in ASSET_NAMES {
        let expected = format!("assets/ipc/{name}");
        if !files.lines().any(|line| line == expected) {
            return Err(format!(
                "desktop crate package is missing {expected}; keep Cargo.toml include rules in sync"
            ));
        }
    }
    if files
        .lines()
        .any(|line| line.starts_with("tests/fixtures/native-ipc/"))
    {
        return Err(
            "desktop crate package contains the repository-only native IPC fixture".to_string(),
        );
    }
    Ok(())
}

fn verify_packaged_consumer(root: &Path) -> Result<(), String> {
    let version = crate::version::read_version()?;
    let package_root = cargo_target_directory(root).join("package");
    let archive = package_root.join(format!("microsoft-webui-desktop-{version}.crate"));
    let temporary = tempfile::tempdir()
        .map_err(|error| format!("failed to create package verification directory: {error}"))?;
    let archive_arg = path_argument(&archive)?;
    let temporary_arg = path_argument(temporary.path())?;
    run_command_quiet("tar", &["-xzf", archive_arg, "-C", temporary_arg], None)
        .map_err(|error| format!("failed to extract {}: {error}", archive.display()))?;

    let extracted = temporary
        .path()
        .join(format!("microsoft-webui-desktop-{version}"));
    let consumer = temporary.path().join("consumer");
    fs::create_dir_all(consumer.join("src"))
        .map_err(|error| format!("failed to create package consumer: {error}"))?;
    fs::write(consumer.join("src/lib.rs"), "")
        .map_err(|error| format!("failed to write package consumer source: {error}"))?;
    let manifest = format!(
        "[package]\nname = \"webui-desktop-package-check\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n\
         [dependencies]\nmicrosoft-webui-desktop = {{ path = {}, default-features = false, features = [\"application-ipc\"] }}\n\n\
         [patch.crates-io]\nmicrosoft-webui-handler = {{ path = {} }}\n\
         microsoft-webui-protocol = {{ path = {} }}\n\
         microsoft-webui-tokens = {{ path = {} }}\n",
        toml_path(&extracted)?,
        toml_path(&root.join("crates/webui-handler"))?,
        toml_path(&root.join("crates/webui-protocol"))?,
        toml_path(&root.join("crates/webui-tokens"))?,
    );
    fs::write(consumer.join("Cargo.toml"), manifest)
        .map_err(|error| format!("failed to write package consumer manifest: {error}"))?;

    let mut command = crate::util::build_command("cargo", &["check", "--quiet"]);
    command
        .current_dir(&consumer)
        .env("CARGO_TARGET_DIR", temporary.path().join("consumer-target"));
    prevent_node_execution(temporary.path(), &mut command)?;
    let output = command
        .output()
        .map_err(|error| format!("failed to build packaged desktop consumer: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "packaged desktop consumer failed to build without Node:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

fn cargo_target_directory(root: &Path) -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(path) if Path::new(&path).is_absolute() => PathBuf::from(path),
        Some(path) => root.join(path),
        None => root.join("target"),
    }
}

fn toml_path(path: &Path) -> Result<String, String> {
    serde_json::to_string(&path.to_string_lossy())
        .map_err(|error| format!("failed to encode path {}: {error}", path.display()))
}

#[cfg(unix)]
fn prevent_node_execution(root: &Path, command: &mut std::process::Command) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    let bin = root.join("no-node");
    fs::create_dir_all(&bin)
        .map_err(|error| format!("failed to create Node guard directory: {error}"))?;
    let node = bin.join("node");
    let mut file =
        fs::File::create(&node).map_err(|error| format!("failed to create Node guard: {error}"))?;
    file.write_all(b"#!/bin/sh\nexit 73\n")
        .map_err(|error| format!("failed to write Node guard: {error}"))?;
    fs::set_permissions(&node, fs::Permissions::from_mode(0o755))
        .map_err(|error| format!("failed to make Node guard executable: {error}"))?;
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let path = std::env::join_paths(paths)
        .map_err(|error| format!("failed to construct Node-free PATH: {error}"))?;
    command.env("PATH", path);
    Ok(())
}

#[cfg(not(unix))]
fn prevent_node_execution(
    _root: &Path,
    _command: &mut std::process::Command,
) -> Result<(), String> {
    Ok(())
}

fn stage_package_directory(generated: &Path, packaged: &Path) -> Result<(), String> {
    remove_package_directory(packaged)?;
    fs::create_dir_all(packaged)
        .map_err(|error| format!("failed to create {}: {error}", packaged.display()))?;
    for name in ASSET_NAMES {
        fs::copy(generated.join(name), packaged.join(name)).map_err(|error| {
            format!(
                "failed to stage packaged desktop asset {}: {error}",
                packaged.join(name).display()
            )
        })?;
    }
    validate_assets(packaged)
}

fn remove_package_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "refusing to remove redirected desktop package asset directory {}",
            path.display()
        )),
        Ok(_) => fs::remove_dir_all(path)
            .map_err(|error| format!("failed to remove {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to inspect {}: {error}", path.display())),
    }
}

fn validate_assets(path: &Path) -> Result<(), String> {
    for name in ASSET_NAMES {
        let asset = path.join(name);
        let metadata = fs::metadata(&asset).map_err(|error| {
            format!("missing desktop browser asset {}: {error}", asset.display())
        })?;
        if metadata.len() == 0 {
            return Err(format!(
                "desktop browser asset {} is empty",
                asset.display()
            ));
        }
    }
    Ok(())
}

fn path_argument(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| format!("desktop asset path is not valid UTF-8: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{stage_package_directory, validate_assets, with_staged_assets, ASSET_NAMES};
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn stages_only_complete_nonempty_asset_sets() {
        let temporary = TempDir::new().unwrap();
        let generated = temporary.path().join("generated");
        let packaged = temporary.path().join("packaged");
        fs::create_dir_all(&generated).unwrap();
        for name in ASSET_NAMES {
            fs::write(generated.join(name), name.as_bytes()).unwrap();
        }

        stage_package_directory(&generated, &packaged).unwrap();
        validate_assets(&packaged).unwrap();
        for name in ASSET_NAMES {
            assert_eq!(fs::read(packaged.join(name)).unwrap(), name.as_bytes());
        }
    }

    #[test]
    fn rejects_incomplete_asset_sets() {
        let temporary = TempDir::new().unwrap();
        fs::write(temporary.path().join(ASSET_NAMES[0]), b"asset").unwrap();
        assert!(validate_assets(temporary.path()).is_err());
    }

    #[test]
    fn staging_failure_removes_partial_package_assets() {
        let temporary = TempDir::new().unwrap();
        let generated = temporary.path().join("generated");
        let packaged = temporary.path().join("packaged");
        fs::create_dir_all(&generated).unwrap();
        fs::write(generated.join(ASSET_NAMES[0]), b"asset").unwrap();

        assert!(with_staged_assets(&generated, &packaged, || Ok(())).is_err());
        assert!(!packaged.exists());
    }
}
