// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Direct SDK downloads built exclusively from the validated canonical SDK.

use super::{sdk, PlatformEntry, PLATFORMS};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn name(version: &str, platform: &PlatformEntry) -> String {
    format!("webui-sdk-{version}-{}.tar.gz", platform.platform_suffix)
}

fn package_version(root: &Path, platform: &PlatformEntry) -> Result<String, String> {
    let path = root
        .join("packages")
        .join(platform.npm_package)
        .join("package.json");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let version = manifest["version"].as_str().ok_or("missing SDK version")?;
    crate::release_version::ReleaseVersion::parse(version).ok_or("invalid SDK release version")?;
    Ok(version.to_string())
}

pub(super) fn stage(root: &Path, platform: &PlatformEntry) -> Result<PathBuf, String> {
    let output = root.join("publish/sdk");
    fs::create_dir_all(&output).map_err(|e| e.to_string())?;
    remove_target_archives(&output, platform)?;
    let version = package_version(root, platform)?;
    let filename = name(&version, platform);
    let archive = output.join(&filename);
    let checksum = output.join(format!("{filename}.sha256"));
    sdk::validate_staged(root, platform)?;
    let source = root.join("packages").join(platform.npm_package).join("sdk");
    for path in [&source, &source.join("include"), &source.join("lib")] {
        if !fs::symlink_metadata(path)
            .map_err(|e| e.to_string())?
            .is_dir()
        {
            return Err(format!(
                "SDK directory must not be a symlink: {}",
                path.display()
            ));
        }
    }
    if !fs::symlink_metadata(source.join("sdk.json"))
        .map_err(|e| e.to_string())?
        .is_file()
    {
        return Err("SDK metadata must be a regular file".to_string());
    }
    let temporary = tempfile::tempdir_in(&output).map_err(|e| e.to_string())?;
    let staged_archive = temporary.path().join(&filename);
    let mut command = Command::new("tar");
    command
        .env("COPYFILE_DISABLE", "1")
        .arg("-czf")
        .arg(&staged_archive)
        .arg("-C")
        .arg(&source)
        .args(["include/webui_ffi.h", "sdk.json", "LICENSE"])
        .arg(format!("lib/{}", platform.ffi_lib));
    if platform.platform_suffix.starts_with("win32-") {
        command.arg("lib/webui_ffi.dll.lib");
    }
    let result = command
        .output()
        .map_err(|e| format!("SDK archive requires tar on PATH: {e}"))?;
    if !result.status.success() {
        return Err(format!(
            "SDK archive failed: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    let digest = sdk::sha256(&staged_archive)?;
    fs::write(
        temporary.path().join("checksum"),
        format!("{digest}  {filename}\n"),
    )
    .map_err(|e| e.to_string())?;
    fs::rename(staged_archive, &archive).map_err(|e| e.to_string())?;
    fs::rename(temporary.path().join("checksum"), checksum).map_err(|e| e.to_string())?;
    Ok(archive)
}

pub(super) fn validate_version(
    root: &Path,
    platform: &PlatformEntry,
    version: &str,
) -> Result<(), String> {
    if package_version(root, platform)? != version {
        return Err(format!(
            "SDK package {} must match release {version}",
            platform.npm_package
        ));
    }
    Ok(())
}

fn remove_target_archives(directory: &Path, platform: &PlatformEntry) -> Result<(), String> {
    let suffix = format!("-{}.tar.gz", platform.platform_suffix);
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let filename = entry.file_name();
        let text = filename.to_string_lossy();
        if text.ends_with(&suffix) || text.ends_with(&format!("{suffix}.sha256")) {
            fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub(super) fn validate_release(root: &Path, version: &str) -> Result<(), String> {
    let output = root.join("publish/sdk");
    if super::count_regular_files(&output) != 12 {
        return Err("expected six SDK archives and six SHA-256 sidecars".to_string());
    }
    for platform in PLATFORMS {
        let filename = name(version, platform);
        let digest = sdk::sha256(&output.join(&filename))?;
        let actual = fs::read_to_string(output.join(format!("{filename}.sha256")))
            .map_err(|e| format!("missing SDK checksum for {filename}: {e}"))?;
        if actual != format!("{digest}  {filename}\n") {
            return Err(format!("SDK archive checksum mismatch: {filename}"));
        }
    }
    Ok(())
}

pub(super) fn export(root: &Path, output: &Path, platform: &PlatformEntry) -> Result<(), String> {
    let source = root.join("publish/sdk");
    let destination = output.join("publish/sdk");
    fs::create_dir_all(&destination).map_err(|e| e.to_string())?;
    // Matrix legs own only their target, never remove another target's archive.
    let filename = name(&package_version(root, platform)?, platform);
    let sidecar = format!("{filename}.sha256");
    let expected = format!("{}  {filename}\n", sdk::sha256(&source.join(&filename))?);
    if fs::read_to_string(source.join(&sidecar)).map_err(|e| e.to_string())? != expected {
        return Err(format!("SDK export checksum mismatch: {filename}"));
    }
    remove_target_archives(&destination, platform)?;
    for filename in [filename, sidecar] {
        fs::copy(source.join(&filename), destination.join(filename)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "publish_archives_tests.rs"]
mod tests;
