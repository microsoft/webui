// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Stage and validate the canonical C ABI SDK shared by npm and direct archives.

use super::PlatformEntry;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Command;

pub(super) fn stage(root: &Path, platform: &PlatformEntry, build_dir: &Path) -> Result<(), String> {
    let package_dir = root.join("packages").join(platform.npm_package);
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(package_dir.join("package.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let version = manifest["version"]
        .as_str()
        .filter(|version| !version.is_empty())
        .ok_or("native package manifest is missing its release version")?;
    let (os, arch) = platform
        .platform_suffix
        .split_once('-')
        .ok_or("invalid native platform suffix")?;
    let sdk = package_dir.join("sdk");
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
        ("package".to_string(), manifest["name"].clone()),
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
    // npm release packaging already requires Node. Use its OpenSSL-backed
    // implementation rather than introducing another Rust crypto consumer.
    const SCRIPT: &str = r#"
        import { createHash } from 'node:crypto';
        import { createReadStream } from 'node:fs';
        try {
            const hash = createHash('sha256');
            for await (const chunk of createReadStream(process.argv[1])) hash.update(chunk);
            process.stdout.write(hash.digest('hex'));
        } catch (error) {
            console.error(error.message);
            process.exitCode = 1;
        }
    "#;
    let output = Command::new("node")
        .args(["--input-type=module", "--eval", SCRIPT, "--"])
        .arg(path)
        .output()
        .map_err(|error| format!("SDK hashing requires Node.js on PATH: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "SDK hashing failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let digest = String::from_utf8(output.stdout).map_err(|error| error.to_string())?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!(
            "Node.js returned an invalid SDK SHA-256 digest for {}",
            path.display()
        ));
    }
    Ok(digest)
}

pub(super) fn validate_staged(root: &Path, platform: &PlatformEntry) -> Result<(), String> {
    let package_dir = root.join("packages").join(platform.npm_package);
    let read_json = |path: &Path| -> Result<serde_json::Value, String> {
        serde_json::from_slice(&fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?)
            .map_err(|e| format!("{}: {e}", path.display()))
    };
    let manifest = read_json(&package_dir.join("package.json"))?;
    let sdk = package_dir.join("sdk");
    let metadata = read_json(&sdk.join("sdk.json"))?;
    let (os, arch) = platform
        .platform_suffix
        .split_once('-')
        .ok_or("invalid platform")?;
    if manifest["version"]
        .as_str()
        .filter(|v| !v.is_empty())
        .is_none()
        || manifest["name"] != format!("@microsoft/{}", platform.npm_package)
        || metadata["schemaVersion"] != 1
        || metadata["version"] != manifest["version"]
        || metadata["package"] != manifest["name"]
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
            platform.npm_package
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
            .map_err(|e| e.to_string())?
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
    use std::process::{Command, Output};

    const HELPER: &str =
        include_str!("../../crates/webui-cli/src/commands/init/templates/common/scripts/sdk.mjs");

    #[test]
    fn node_hashes_known_vectors_and_reports_missing_files() {
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
        assert!(error.contains("SDK hashing failed"));
        assert!(error.contains("ENOENT"));
    }

    fn helper(project: &Path) -> std::path::PathBuf {
        let script = project.join("sdk.mjs");
        fs::write(&script, HELPER.replace("__WEBUI_VERSION__", "3.1.2")).expect("SDK helper");
        script
    }

    fn node(project: &Path, sdk: Option<&Path>) -> Output {
        let mut command = Command::new("node");
        command
            .arg(helper(project))
            .current_dir(project)
            .env_remove("WEBUI_SDK_DIR");
        if let Some(sdk) = sdk {
            command.env("WEBUI_SDK_DIR", sdk);
        }
        command
            .output()
            .expect("Node is required to exercise the generated SDK helper")
    }

    fn host_platform() -> &'static PlatformEntry {
        let triple = crate::publish::detect_host_triple();
        PLATFORMS
            .iter()
            .find(|platform| platform.triple == triple)
            .expect("supported host")
    }

    fn staged_fixture(root: &Path) -> std::path::PathBuf {
        let platform = host_platform();
        fixture(root, platform);
        let build = root.join("build");
        fs::create_dir_all(&build).expect("build");
        fs::write(build.join(platform.ffi_lib), "ffi").expect("FFI fixture");
        fs::write(build.join("webui_ffi.dll.lib"), "import").expect("import fixture");
        stage(root, platform, &build).expect("staged SDK fixture");
        root.join("packages").join(platform.npm_package).join("sdk")
    }

    fn assert_error(output: Output, message: &str) {
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(message), "expected {message:?}: {stderr}");
    }

    #[test]
    fn helper_resolves_offline_sdk_and_rejects_missing_incompatible_or_corrupt_sdk() {
        let root = tempfile::tempdir().expect("workspace");
        let project = tempfile::Builder::new()
            .prefix("sdk app with spaces ")
            .tempdir()
            .expect("app");
        assert_error(node(project.path(), None), "Missing @microsoft/webui-");
        let sdk = staged_fixture(root.path());
        let output = node(project.path(), Some(&sdk));
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            fs::canonicalize(&sdk).expect("SDK root").to_string_lossy()
        );
        let metadata_path = sdk.join("sdk.json");
        let original = fs::read(&metadata_path).expect("metadata");
        let mut metadata: serde_json::Value = serde_json::from_slice(&original).expect("JSON");
        metadata["version"] = json!("0.0.0");
        fs::write(&metadata_path, metadata.to_string()).expect("wrong version");
        assert_error(node(project.path(), Some(&sdk)), "version mismatch");
        metadata["version"] = json!("3.1.2");
        metadata["target"] = json!("wrong-target");
        fs::write(&metadata_path, metadata.to_string()).expect("wrong target");
        assert_error(node(project.path(), Some(&sdk)), "platform mismatch");
        fs::write(&metadata_path, original).expect("restore metadata");
        fs::write(sdk.join("include/webui_ffi.h"), "tampered").expect("tamper");
        assert_error(node(project.path(), Some(&sdk)), "integrity check failed");
        fs::remove_file(sdk.join("include/webui_ffi.h")).expect("remove header");
        assert_error(node(project.path(), Some(&sdk)), "ENOENT");
    }

    #[test]
    fn helper_resolves_installed_optional_package_and_rejects_wrong_package_version() {
        let root = tempfile::tempdir().expect("workspace");
        let project = tempfile::tempdir().expect("app");
        let sdk = staged_fixture(root.path());
        let package = project
            .path()
            .join("node_modules/@microsoft")
            .join(host_platform().npm_package);
        crate::publish::copy_directory_contents(sdk.parent().expect("package"), &package)
            .expect("install fixture");
        let output = node(project.path(), None);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        fs::write(
            package.join("package.json"),
            json!({
                "name": format!("@microsoft/{}", host_platform().npm_package),
                "version": "0.0.0",
            })
            .to_string(),
        )
        .expect("mismatched installed package");
        assert_error(
            node(project.path(), None),
            "native package version mismatch",
        );
    }

    #[test]
    fn helper_resolves_optional_package_nested_beside_compiler() {
        let root = tempfile::tempdir().expect("workspace");
        let project = tempfile::tempdir().expect("app");
        let sdk = staged_fixture(root.path());
        let compiler = project.path().join("node_modules/@microsoft/webui");
        fs::create_dir_all(&compiler).expect("compiler");
        fs::write(compiler.join("package.json"), "{}").expect("compiler manifest");
        let package = compiler
            .join("node_modules/@microsoft")
            .join(host_platform().npm_package);
        crate::publish::copy_directory_contents(sdk.parent().expect("package"), &package)
            .expect("install optional fixture");
        let output = node(project.path(), None);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn helper_sets_cgo_and_loader_environment_without_shell_interpolation() {
        let project = tempfile::Builder::new()
            .prefix("sdk app with spaces ")
            .tempdir()
            .expect("app");
        let script = helper(project.path());
        let output = Command::new("node").args(["--input-type=module", "-e", r#"
            import assert from 'node:assert/strict';
            import path from 'node:path';
            import { pathToFileURL } from 'node:url';
            const { sdkEnvironment, provisionSdk } = await import(pathToFileURL(process.argv[2]));
            const env = sdkEnvironment(process.cwd(), { CGO_CFLAGS: '-DTEST', CGO_LDFLAGS: '-lm' });
            assert.equal(env.WEBUI_SDK_DIR, process.cwd());
            assert.equal(env.CGO_ENABLED, '1');
            assert.ok(env.CGO_CFLAGS.startsWith('-DTEST "-I'));
            assert.ok(env.CGO_LDFLAGS.startsWith('-lm "-L'));
            const loader = process.platform === 'win32' ? 'PATH' : process.platform === 'darwin' ? 'DYLD_LIBRARY_PATH' : 'LD_LIBRARY_PATH';
            assert.equal(env[loader], path.join(process.cwd(), 'lib'));
            Object.defineProperty(process, 'arch', { value: 'unsupported' });
            await assert.rejects(() => provisionSdk(), /does not support/);
        "#]).arg("environment-test").arg(script).current_dir(project.path()).output().expect("Node");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    pub(in crate::publish) fn fixture(root: &Path, platform: &PlatformEntry) {
        let package = root.join("packages").join(platform.npm_package);
        fs::create_dir_all(&package).expect("package directory");
        fs::write(
            package.join("package.json"),
            json!({
                "name": format!("@microsoft/{}", platform.npm_package),
                "version": "3.1.2",
            })
            .to_string(),
        )
        .expect("package manifest");
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
            let sdk = root
                .path()
                .join("packages")
                .join(platform.npm_package)
                .join("sdk");
            let metadata: serde_json::Value =
                serde_json::from_slice(&fs::read(sdk.join("sdk.json")).expect("metadata"))
                    .expect("valid metadata");
            assert_eq!(metadata["version"], "3.1.2");
            assert_eq!(metadata["target"], platform.triple);
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
            .join("packages")
            .join(platform.npm_package)
            .join("sdk/sdk.json")
            .exists());
    }
}
