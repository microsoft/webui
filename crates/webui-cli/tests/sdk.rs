// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use anyhow::{Context, Result};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

fn cli(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_webui"));
    command
        .args(["--format", "json", "sdk"])
        .current_dir(root)
        .env_remove("WEBUI_SDK_DIR");
    command
}

// These bytes exercise inventory validation only, not native library loading.
fn inventory(root: &Path) -> Result<Value> {
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    let cpu = std::env::consts::ARCH;
    let (os, suffix, library) = if cfg!(target_os = "macos") {
        ("darwin", "apple-darwin", "libwebui_ffi.dylib")
    } else if cfg!(windows) {
        ("win32", "pc-windows-msvc", "webui_ffi.dll")
    } else {
        ("linux", "unknown-linux-gnu", "libwebui_ffi.so")
    };
    let mut files = Map::new();
    let paths = [
        "include/webui_ffi.h".to_owned(),
        format!("lib/{library}"),
        "LICENSE".to_owned(),
    ];
    for relative in paths
        .iter()
        .map(String::as_str)
        .chain(cfg!(windows).then_some("lib/webui_ffi.dll.lib"))
    {
        let file = root.join(relative);
        fs::create_dir_all(file.parent().context("inventory parent")?)?;
        fs::write(&file, relative)?;
        let bytes = Sha256::digest(relative.as_bytes());
        let mut hex = String::with_capacity(64);
        use std::fmt::Write;
        for byte in bytes {
            write!(hex, "{byte:02x}")?;
        }
        files.insert(relative.to_owned(), Value::from(hex));
    }
    let mut metadata = Map::new();
    for (name, value) in [
        ("schemaVersion", Value::from(1)),
        ("version", Value::from(env!("CARGO_PKG_VERSION"))),
        ("target", Value::from(format!("{cpu}-{suffix}"))),
        ("os", Value::from(os)),
        ("arch", Value::from(arch)),
        (
            "libc",
            if cfg!(target_os = "linux") {
                Value::from("gnu")
            } else {
                Value::Null
            },
        ),
        ("files", Value::Object(files)),
    ] {
        metadata.insert(name.to_owned(), value);
    }
    let value = Value::Object(metadata);
    write_metadata(root, &value)?;
    Ok(value)
}

fn write_metadata(root: &Path, metadata: &Value) -> Result<()> {
    fs::write(root.join("sdk.json"), serde_json::to_vec(metadata)?)?;
    Ok(())
}

fn error(output: &Output, code: &str, status: i32) -> Result<()> {
    assert_eq!(
        output.status.code(),
        Some(status),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let record: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(record["code"], code);
    assert!(record["help"].as_str().context("actionable help")?.len() > 20);
    assert!(!output.stdout.contains(&0x1b));
    Ok(())
}

#[test]
fn checks_project_local_sdk_without_node_and_rejects_integrity_failures() -> Result<()> {
    let temp = TempDir::new()?;
    let sdk = temp.path().join(".webui/sdk");
    let mut metadata = inventory(&sdk)?;
    let checked = cli(temp.path()).args(["check"]).env("PATH", "").output()?;
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stdout)
    );
    let report: Value = serde_json::from_slice(&checked.stdout)?;
    assert_eq!(report["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(report["target"], metadata["target"]);
    assert_eq!(
        report["directory"],
        sdk.canonicalize()?.to_str().context("SDK path")?
    );
    fs::write(sdk.join("include/webui_ffi.h"), "corrupted header")?;
    error(&cli(temp.path()).arg("check").output()?, "sdk-invalid", 65)?;
    inventory(&sdk)?;
    metadata["target"] = Value::from("incompatible-target");
    write_metadata(&sdk, &metadata)?;
    error(&cli(temp.path()).arg("check").output()?, "sdk-invalid", 65)?;
    metadata = inventory(&sdk)?;
    metadata["version"] = Value::from("0.0.0");
    write_metadata(&sdk, &metadata)?;
    error(&cli(temp.path()).arg("check").output()?, "sdk-invalid", 65)?;
    metadata = inventory(&sdk)?;
    metadata["files"]["../../outside"] = Value::from("0".repeat(64));
    write_metadata(&sdk, &metadata)?;
    error(&cli(temp.path()).arg("check").output()?, "sdk-invalid", 65)?;
    Ok(())
}

#[test]
fn explicit_override_never_falls_back_to_a_valid_project_sdk() -> Result<()> {
    let temp = TempDir::new()?;
    inventory(&temp.path().join(".webui/sdk"))?;
    error(
        &cli(temp.path())
            .arg("check")
            .env("WEBUI_SDK_DIR", "")
            .output()?,
        "sdk-invalid",
        65,
    )?;
    error(
        &cli(temp.path())
            .arg("check")
            .env("WEBUI_SDK_DIR", temp.path().join("missing"))
            .output()?,
        "sdk-read-failed",
        74,
    )?;
    let external = temp.path().join("external SDK");
    inventory(&external)?;
    let result = cli(temp.path())
        .arg("check")
        .env("WEBUI_SDK_DIR", &external)
        .output()?;
    assert!(result.status.success());
    Ok(())
}

#[test]
fn validates_every_sdk_identity_field_and_never_resolves_npm_payloads() -> Result<()> {
    let temp = TempDir::new()?;
    let os = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(windows) {
        "win32"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    let package = format!("@microsoft/webui-{os}-{arch}");
    let npm = temp.path().join("node_modules").join(&package);
    inventory(&npm.join("sdk"))?;
    let mut manifest = Map::new();
    manifest.insert("name".into(), Value::from(package));
    manifest.insert("version".into(), Value::from(env!("CARGO_PKG_VERSION")));
    fs::write(npm.join("package.json"), serde_json::to_vec(&manifest)?)?;
    error(
        &cli(temp.path()).arg("check").output()?,
        "sdk-read-failed",
        74,
    )?;
    let sdk = temp.path().join(".webui/sdk");
    let metadata = inventory(&sdk)?;
    for (key, value) in [
        ("schemaVersion", Value::from(0)),
        ("version", Value::from("0.0.0")),
        ("target", Value::from("wrong")),
        ("os", Value::from("wrong")),
        ("arch", Value::from("wrong")),
        ("files", Value::Object(Map::new())),
    ] {
        let mut invalid = metadata.clone();
        invalid[key] = value;
        write_metadata(&sdk, &invalid)?;
        error(&cli(temp.path()).arg("check").output()?, "sdk-invalid", 65)?;
    }
    #[cfg(target_os = "linux")]
    {
        let mut invalid = metadata.clone();
        invalid["libc"] = Value::from("musl");
        write_metadata(&sdk, &invalid)?;
        error(&cli(temp.path()).arg("check").output()?, "sdk-invalid", 65)?;
    }
    Ok(())
}

#[test]
fn exec_sets_native_paths_preserves_flags_and_propagates_exit_status() -> Result<()> {
    let temp = tempfile::Builder::new()
        .prefix("native SDK with spaces ")
        .tempdir()?;
    let sdk = temp.path().join(".webui/sdk");
    inventory(&sdk)?;
    let output = cli(temp.path())
        .args(["exec", "--"])
        .arg(std::env::current_exe()?)
        .args(["--exact", "sdk_environment_child", "--nocapture"])
        .env("WEBUI_SDK_TEST_CHILD", "1")
        .env("CGO_CFLAGS", "-DEXISTING_C")
        .env("CGO_CPPFLAGS", "-DEXISTING_CPP")
        .env("CGO_LDFLAGS", "-DEXISTING_LD")
        .env("PATH", "")
        .output()?;
    assert_eq!(
        output.status.code(),
        Some(23),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    error(
        &cli(temp.path())
            .args(["exec", "--", "no-such-sdk-command;echo"])
            .output()?,
        "sdk-command-failed",
        1,
    )?;
    Ok(())
}

#[test]
fn sdk_environment_child() -> Result<()> {
    if std::env::var_os("WEBUI_SDK_TEST_CHILD").is_none() {
        return Ok(());
    }
    let root = std::env::var("WEBUI_SDK_DIR")?;
    let normalized = if let Some(unc) = root.strip_prefix(r"\\?\UNC\") {
        format!("//{}", unc.replace('\\', "/"))
    } else {
        root.strip_prefix(r"\\?\")
            .unwrap_or(&root)
            .replace('\\', "/")
    };
    assert_eq!(std::env::var("CGO_ENABLED")?, "1");
    for (name, inherited, prefix) in [
        ("CGO_CFLAGS", "-DEXISTING_C", "-I"),
        ("CGO_CPPFLAGS", "-DEXISTING_CPP", "-I"),
        ("CGO_LDFLAGS", "-DEXISTING_LD", "-L"),
    ] {
        let flag = std::env::var(name)?;
        assert!(flag.contains(&normalized));
        assert!(flag.starts_with(&format!("\"{prefix}")));
        assert!(flag.ends_with(inherited));
    }
    std::process::exit(23);
}

#[cfg(unix)]
#[test]
fn refuses_symlinked_inventory_files_and_unbounded_metadata() -> Result<()> {
    let temp = TempDir::new()?;
    let sdk = temp.path().join(".webui/sdk");
    inventory(&sdk)?;
    fs::remove_file(sdk.join("LICENSE"))?;
    fs::write(temp.path().join("outside-license"), "LICENSE")?;
    std::os::unix::fs::symlink(temp.path().join("outside-license"), sdk.join("LICENSE"))?;
    error(&cli(temp.path()).arg("check").output()?, "sdk-invalid", 65)?;
    fs::write(sdk.join("sdk.json"), vec![b' '; 65_537])?;
    error(&cli(temp.path()).arg("check").output()?, "sdk-invalid", 65)?;
    Ok(())
}
