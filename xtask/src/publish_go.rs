// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Source-only, offline-testable Go proxy artifacts. No tags or network writes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const MODULE: &str = "github.com/microsoft/webui/go/v3";
const PACKER: &str = include_str!("publish_go_packer.go");

#[cfg(test)]
fn proxy(root: &Path) -> PathBuf {
    proxy_at(&root.join("publish/go"))
}

fn proxy_at(output: &Path) -> PathBuf {
    output.join(MODULE).join("@v")
}

fn check_version(version: &str) -> Result<String, String> {
    let parsed = crate::release_version::ReleaseVersion::parse(version);
    if parsed.is_none() || !version.starts_with("3.") {
        return Err(format!(
            "Go module {MODULE} requires a release-aligned v3 version, got {version}"
        ));
    }
    parsed
        .and_then(crate::release_version::ReleaseVersion::go_version)
        .ok_or_else(|| format!("Go hotfix mapping requires a patch below u64::MAX: {version}"))
}

pub(super) fn stage(root: &Path, version: &str) -> Result<(), String> {
    stage_at(root, version, &root.join("publish/go"))
}

pub(super) fn stage_at(root: &Path, version: &str, output: &Path) -> Result<(), String> {
    let go_version = check_version(version)?;
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let destination = proxy_at(output);
    // Do not retain earlier proxy artifacts after a failed source validation.
    if destination.exists() {
        fs::remove_dir_all(&destination).map_err(|e| e.to_string())?;
    }
    let temporary = tempfile::tempdir().map_err(|e| e.to_string())?;
    let helper = temporary.path().join("pack.go");
    fs::write(&helper, PACKER).map_err(|e| e.to_string())?;
    let packed = Command::new("go")
        .arg("run")
        .arg(&helper)
        .arg(root.join("go"))
        .arg(root.join("LICENSE"))
        .arg(temporary.path().join("proxy"))
        .arg(version)
        .arg(go_version)
        .env("GOWORK", "off")
        .env("GOTOOLCHAIN", "local")
        .env("GOFLAGS", "")
        .current_dir(temporary.path())
        .output()
        .map_err(|e| format!("Go source packaging requires Go on PATH: {e}"))?;
    if !packed.status.success() {
        return Err(format!(
            "Go source packaging failed: {}",
            String::from_utf8_lossy(&packed.stderr).trim()
        ));
    }
    fs::create_dir_all(&destination).map_err(|e| e.to_string())?;
    super::copy_directory_contents(&temporary.path().join("proxy"), &destination)?;
    validate_at(output, version)
}

pub(super) fn validate_release(root: &Path, version: &str) -> Result<(), String> {
    validate_at(&root.join("publish/go"), version)
}

fn validate_at(output: &Path, version: &str) -> Result<(), String> {
    let version = check_version(version)?;
    let output = proxy_at(output);
    if super::count_regular_files(&output) != 4 {
        return Err("expected Go proxy .mod, .info, .zip and list artifacts".to_string());
    }
    for extension in ["mod", "info", "zip"] {
        if !output.join(format!("v{version}.{extension}")).is_file() {
            return Err(format!("missing Go proxy artifact v{version}.{extension}"));
        }
    }
    let info: serde_json::Value = serde_json::from_slice(
        &fs::read(output.join(format!("v{version}.info"))).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if info["Version"] != format!("v{version}")
        || fs::read_to_string(output.join("list")).map_err(|e| e.to_string())?
            != format!("v{version}\n")
    {
        return Err("Go proxy version identity mismatch".to_string());
    }
    Ok(())
}

#[cfg(test)]
#[path = "publish_go_tests.rs"]
mod tests;
