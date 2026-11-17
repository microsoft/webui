// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Stage and validate the standalone, language-neutral C ABI SDK.

use super::PlatformEntry;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

pub(super) fn directory(root: &Path, platform: &PlatformEntry) -> PathBuf {
    root.join("publish/sdk-contents").join(platform.triple)
}

pub(super) fn release_version(root: &Path) -> Result<String, String> {
    let version = crate::version::workspace_package_version(root)?;
    crate::release_version::ReleaseVersion::parse(&version).ok_or("invalid SDK release version")?;
    Ok(version)
}

pub(super) fn stage(root: &Path, platform: &PlatformEntry, build_dir: &Path) -> Result<(), String> {
    let version = release_version(root)?;
    let (os, arch) = platform
        .platform_suffix
        .split_once('-')
        .ok_or("invalid native platform suffix")?;
    let sdk = directory(root, platform);
    // Never leave stale libraries or metadata in a subsequently packed SDK.
    if sdk.exists() {
        fs::remove_dir_all(&sdk).map_err(|e| e.to_string())?;
    }
    let mut files = vec![
        (
            root.join("crates/webui-ffi/include/webui_ffi.h"),
            "include/webui_ffi.h".to_string(),
        ),
        (
            build_dir.join(platform.ffi_lib),
            format!("lib/{}", platform.ffi_lib),
        ),
        (root.join("LICENSE"), "LICENSE".to_string()),
    ];
    if os == "win32" {
        files.push((
            build_dir.join("webui_ffi.dll.lib"),
            "lib/webui_ffi.dll.lib".to_string(),
        ));
    }
    let mut hashes = serde_json::Map::new();
    for (source, relative) in files {
        let destination = sdk.join(&relative);
        let parent = destination.parent().ok_or("invalid SDK destination")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        fs::copy(&source, &destination)
            .map_err(|e| format!("cannot stage {}: {e}", source.display()))?;
        hashes.insert(relative, Value::from(sha256(&destination)?));
    }
    let metadata = Value::Object(serde_json::Map::from_iter([
        ("schemaVersion".to_string(), Value::from(1)),
        ("version".to_string(), Value::from(version)),
        ("target".to_string(), Value::from(platform.triple)),
        ("os".to_string(), Value::from(os)),
        ("arch".to_string(), Value::from(arch)),
        (
            "libc".to_string(),
            if os == "linux" {
                Value::from("gnu")
            } else {
                Value::Null
            },
        ),
        ("files".to_string(), Value::Object(hashes)),
    ]));
    fs::write(
        sdk.join("sdk.json"),
        serde_json::to_vec_pretty(&metadata).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

pub(super) fn sha256(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut buffer = vec![0_u8; 16 * 1024];
    let mut hash = Sha256::new();
    loop {
        let length = file
            .read(&mut buffer)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if length == 0 {
            break;
        }
        hash.update(&buffer[..length]);
    }
    let mut digest = String::with_capacity(64);
    use std::fmt::Write;
    for byte in hash.finalize() {
        write!(digest, "{byte:02x}").map_err(|error| error.to_string())?;
    }
    Ok(digest)
}

pub(super) fn validate_staged(root: &Path, platform: &PlatformEntry) -> Result<(), String> {
    let read_json = |path: &Path| -> Result<serde_json::Value, String> {
        serde_json::from_slice(&fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?)
            .map_err(|e| format!("{}: {e}", path.display()))
    };
    let version = release_version(root)?;
    let sdk = directory(root, platform);
    let metadata = read_json(&sdk.join("sdk.json"))?;
    let (os, arch) = platform
        .platform_suffix
        .split_once('-')
        .ok_or("invalid platform")?;
    if metadata["schemaVersion"] != 1
        || metadata["version"] != version
        || metadata["target"] != platform.triple
        || metadata["os"] != os
        || metadata["arch"] != arch
        || metadata["libc"]
            != if os == "linux" {
                Value::from("gnu")
            } else {
                Value::Null
            }
    {
        return Err(format!(
            "invalid or version-mismatched SDK metadata for {}",
            platform.triple
        ));
    }
    let mut files = vec![
        "include/webui_ffi.h".to_string(),
        format!("lib/{}", platform.ffi_lib),
        "LICENSE".to_string(),
    ];
    if os == "win32" {
        files.push("lib/webui_ffi.dll.lib".to_string());
    }
    let hashes = metadata["files"]
        .as_object()
        .ok_or("missing SDK file inventory")?;
    if hashes.len() != files.len() {
        return Err("invalid SDK file inventory".to_string());
    }
    for relative in files {
        let path = sdk.join(&relative);
        if !fs::symlink_metadata(&path)
            .map_err(|error| {
                format!(
                    "cannot inspect SDK input {} for {}: {error}; stage this target again",
                    path.display(),
                    platform.triple
                )
            })?
            .is_file()
        {
            return Err(format!("SDK input must be a regular file: {relative}"));
        }
        let expected = hashes.get(&relative).and_then(serde_json::Value::as_str);
        if expected != Some(sha256(&path)?.as_str()) {
            return Err(format!("SDK checksum mismatch: {relative}"));
        }
    }
    // A previous release's header/license must not survive matrix export merging.
    for relative in ["include/webui_ffi.h", "LICENSE"] {
        let source = if relative == "LICENSE" {
            root.join("LICENSE")
        } else {
            root.join("crates/webui-ffi").join(relative)
        };
        if sha256(&source)? != sha256(&sdk.join(relative))? {
            return Err(format!(
                "stale SDK input: {relative}; stage the current release SDK again"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::publish::PLATFORMS;
    use serde_json::json;

    #[test]
    fn missing_staged_sdk_file_reports_path_target_and_recovery() {
        let root = tempfile::tempdir_in(".").expect("workspace");
        let platform = &PLATFORMS[0];
        fixture(root.path(), platform);
        let build = root.path().join("build");
        fs::create_dir_all(&build).expect("build");
        fs::write(build.join(platform.ffi_lib), "ffi").expect("FFI");
        stage(root.path(), platform, &build).expect("SDK");
        let missing = directory(root.path(), platform).join("include/webui_ffi.h");
        fs::remove_file(&missing).expect("remove staged header");
        let error = validate_staged(root.path(), platform).expect_err("missing SDK input");
        assert!(error.contains(&missing.display().to_string()), "{error}");
        assert!(error.contains(platform.triple), "{error}");
        assert!(
            error.contains("stage") && error.contains("again"),
            "{error}"
        );
    }

    #[test]
    fn native_hashes_known_vectors_and_reports_missing_files() {
        let root = tempfile::Builder::new()
            .prefix("SDK hashes with spaces ")
            .tempdir()
            .expect("hash fixture");
        let file = root.path().join("input with spaces.bin");
        fs::write(&file, b"abc").expect("known input");
        assert_eq!(
            sha256(&file).expect("SHA-256 of abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        fs::write(&file, b"").expect("empty input");
        assert_eq!(
            sha256(&file).expect("SHA-256 of empty input"),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        fs::remove_file(&file).expect("remove input");
        let error = sha256(&file).expect_err("missing input must fail");
        assert!(error.contains(&file.display().to_string()), "{error}");
    }

    pub(in crate::publish) fn fixture(root: &Path, _platform: &PlatformEntry) {
        fs::write(
            root.join("Cargo.toml"),
            "[workspace.package]\nversion = \"3.1.2\"\n",
        )
        .expect("workspace manifest");
        let header = root.join("crates/webui-ffi/include");
        fs::create_dir_all(&header).expect("header directory");
        fs::write(header.join("webui_ffi.h"), "header").expect("header");
        fs::write(root.join("LICENSE"), "license").expect("license");
    }

    #[test]
    fn stages_sdk_shape_and_hashes_for_every_release_platform() {
        for platform in PLATFORMS {
            let root = tempfile::tempdir().expect("workspace");
            fixture(root.path(), platform);
            let build = root.path().join("build");
            fs::create_dir_all(&build).expect("build");
            fs::write(build.join(platform.ffi_lib), "native ffi").expect("library");
            fs::write(build.join("webui_ffi.dll.lib"), "import").expect("import library");
            stage(root.path(), platform, &build).expect("SDK staging");
            validate_staged(root.path(), platform).expect("valid staged SDK");
            let sdk = directory(root.path(), platform);
            let metadata: serde_json::Value =
                serde_json::from_slice(&fs::read(sdk.join("sdk.json")).expect("metadata"))
                    .expect("valid metadata");
            assert_eq!(metadata["version"], "3.1.2");
            assert_eq!(metadata["target"], platform.triple);
            assert!(metadata.get("package").is_none());
            assert!(!root.path().join("packages").exists());
            let files = metadata["files"].as_object().expect("files");
            assert_eq!(
                files.len(),
                if platform.platform_suffix.starts_with("win32") {
                    4
                } else {
                    3
                }
            );
            for (relative, hash) in files {
                assert_eq!(hash, &sha256(&sdk.join(relative)).expect("hash"));
            }
            fs::write(sdk.join("include/webui_ffi.h"), "corrupted header").expect("corrupt SDK");
            assert!(validate_staged(root.path(), platform).is_err());
        }
    }

    #[test]
    fn rejects_wrong_release_platform_and_inventory_without_npm_metadata() {
        let root = tempfile::tempdir().expect("workspace");
        let platform = &PLATFORMS[0];
        fixture(root.path(), platform);
        let build = root.path().join("build");
        fs::create_dir_all(&build).expect("build");
        fs::write(build.join(platform.ffi_lib), "ffi").expect("FFI");
        stage(root.path(), platform, &build).expect("SDK");
        let path = directory(root.path(), platform).join("sdk.json");
        let original: Value =
            serde_json::from_slice(&fs::read(&path).expect("metadata")).expect("JSON");
        for (field, wrong) in [
            ("version", json!("03.1.2")),
            ("target", json!("aarch64-unknown-linux-gnu")),
            ("os", json!("darwin")),
            ("arch", json!("arm64")),
            ("libc", Value::Null),
            ("files", json!({})),
        ] {
            let mut metadata = original.clone();
            metadata[field] = wrong;
            fs::write(&path, metadata.to_string()).expect("wrong metadata");
            assert!(validate_staged(root.path(), platform).is_err(), "{field}");
        }
        fs::write(&path, original.to_string()).expect("restore");
        fs::write(
            root.path().join("Cargo.toml"),
            "[workspace.package]\nversion = \"03.1.2\"\n",
        )
        .expect("invalid version");
        assert!(validate_staged(root.path(), platform).is_err());
    }

    #[test]
    fn windows_sdk_requires_import_library_and_never_retains_stale_metadata() {
        let root = tempfile::tempdir().expect("workspace");
        let platform = &PLATFORMS[2];
        fixture(root.path(), platform);
        let build = root.path().join("build");
        fs::create_dir_all(&build).expect("build");
        fs::write(build.join(platform.ffi_lib), "dll").expect("dll");
        assert!(stage(root.path(), platform, &build).is_err());
        fs::write(build.join("webui_ffi.dll.lib"), "import").expect("import library");
        stage(root.path(), platform, &build).expect("SDK staging");
        fs::remove_file(build.join("webui_ffi.dll.lib")).expect("remove import");
        assert!(stage(root.path(), platform, &build).is_err());
        assert!(!root
            .path()
            .join("publish/sdk-contents")
            .join(platform.triple)
            .join("sdk.json")
            .exists());
    }
}
