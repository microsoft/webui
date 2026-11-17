// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;
use serde_json::{Map, Value};
use std::error::Error;
use std::fs;

fn fixture() -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let target = contract::target(env!("WEBUI_NATIVE_TARGET")).ok_or("unsupported test host")?;
    let mut files = Map::new();
    for relative in contract::inventory(&target) {
        let path = temp.path().join(&relative);
        fs::create_dir_all(path.parent().ok_or("missing parent")?)?;
        // Synthetic files exercise validation failures, never native compilation.
        fs::write(&path, b"synthetic validation fixture")?;
        files.insert(relative, Value::from(contract::sha256(&path, 1024)?));
    }
    let metadata = Value::Object(Map::from_iter([
        ("schemaVersion".to_owned(), Value::from(1)),
        ("version".to_owned(), Value::from(env!("CARGO_PKG_VERSION"))),
        (
            "target".to_owned(),
            Value::from(env!("WEBUI_NATIVE_TARGET")),
        ),
        ("typescriptVersion".to_owned(), Value::from("7.0.2")),
        ("files".to_owned(), Value::Object(files)),
    ]));
    fs::write(
        temp.path().join("toolchain.json"),
        serde_json::to_vec(&metadata)?,
    )?;
    Ok(temp)
}

fn change(root: &Path, edit: impl FnOnce(&mut Value)) -> Result<(), Box<dyn std::error::Error>> {
    let path = root.join("toolchain.json");
    let mut value = serde_json::from_slice(&fs::read(&path)?)?;
    edit(&mut value);
    fs::write(path, serde_json::to_vec(&value)?)?;
    Ok(())
}

fn failure(root: &Path) -> ToolchainError {
    match validation::validate(root) {
        Ok(_) => panic!("synthetic fixtures must never validate as native tooling"),
        Err(error) => error,
    }
}

#[test]
fn identity_is_exact_and_errors_are_actionable() -> Result<(), Box<dyn std::error::Error>> {
    for (field, value) in [
        ("schemaVersion", Value::from(2)),
        ("version", Value::from("0.0.0")),
        ("target", Value::from("wrong-target")),
        ("typescriptVersion", Value::from("3.1.2")),
    ] {
        let temp = fixture()?;
        change(temp.path(), |metadata| metadata[field] = value)?;
        let error = failure(temp.path());
        assert_eq!(error.code(), "toolchain-invalid");
        assert_eq!(error.exit_code(), 65);
        assert!(error.to_string().contains("expected schema 1"));
        assert!(error.help().contains("WEBUI_TOOLCHAIN_DIR"));
        assert!(!error.help().contains('\u{1b}'));
    }
    assert_eq!(
        ToolchainError::Unsupported("test".to_owned()).exit_code(),
        69
    );
    assert_eq!(
        ToolchainError::Unsupported("test".to_owned()).code(),
        "toolchain-unsupported-target"
    );
    let error = ToolchainError::Compiler {
        path: PathBuf::new(),
        reason: "test".to_owned(),
    };
    assert_eq!(error.code(), "toolchain-compiler-failed");
    assert_eq!(error.exit_code(), 65);
    Ok(())
}

#[test]
fn override_is_authoritative_even_when_empty() -> Result<(), Box<dyn std::error::Error>> {
    for value in [OsStr::new(""), OsStr::new("missing-explicit-toolchain")] {
        let error = resolve_with(Some(value), || {
            panic!("an explicit override must not inspect the current executable")
        })
        .err()
        .ok_or("unexpected success")?;
        if value.is_empty() {
            assert_eq!(error.code(), "toolchain-invalid");
        } else {
            assert_eq!(error.code(), "toolchain-read-failed");
            assert_eq!(error.exit_code(), 74);
        }
    }
    let temp = fixture()?;
    let error = resolve_with(None, || Ok(temp.path().join("bin/webui")))
        .err()
        .ok_or("unexpected native synthetic fixture")?;
    // Found adjacent root, validated its hashes, and rejected synthetic binaries.
    assert!(error.to_string().contains("native executable"));
    Ok(())
}

#[test]
fn rejects_unknown_escaping_missing_and_duplicate_inventory(
) -> Result<(), Box<dyn std::error::Error>> {
    for relative in [
        "../escape",
        "/absolute",
        "compiler/../escape",
        "compiler\\tsc",
        "unknown",
    ] {
        let temp = fixture()?;
        change(temp.path(), |metadata| {
            metadata["files"][relative] = Value::from("0".repeat(64))
        })?;
        assert!(failure(temp.path()).to_string().contains("inventory"));
    }
    let temp = fixture()?;
    change(temp.path(), |metadata| {
        metadata["files"]
            .as_object_mut()
            .map(|files| files.remove("compiler/lib.dom.d.ts"));
    })?;
    assert!(failure(temp.path()).to_string().contains("inventory"));
    let temp = fixture()?;
    let path = temp.path().join("toolchain.json");
    let text = fs::read_to_string(&path)?;
    let duplicate = text.replacen(
        "\"files\":{",
        "\"files\":{\"compiler/lib.d.ts\":\"duplicate\",",
        1,
    );
    fs::write(path, duplicate)?;
    assert!(failure(temp.path())
        .to_string()
        .contains("duplicate inventory"));
    Ok(())
}

#[test]
fn rejects_corruption_missing_libraries_and_extra_disk_files(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp = fixture()?;
    fs::write(temp.path().join("compiler/lib.dom.d.ts"), b"corrupted")?;
    assert!(failure(temp.path()).to_string().contains("integrity"));
    let temp = fixture()?;
    fs::remove_file(temp.path().join("compiler/lib.dom.d.ts"))?;
    assert_eq!(failure(temp.path()).code(), "toolchain-read-failed");
    let temp = fixture()?;
    fs::write(temp.path().join("compiler/tsc.js"), b"process.exit(0)")?;
    assert!(failure(temp.path()).to_string().contains("unknown entry"));
    let temp = fixture()?;
    change(temp.path(), |metadata| {
        metadata["files"]["compiler/lib.d.ts"] = Value::from("Z".repeat(64))
    })?;
    assert!(failure(temp.path()).to_string().contains("invalid SHA-256"));
    Ok(())
}

#[test]
fn metadata_and_hashing_are_bounded() -> Result<(), Box<dyn std::error::Error>> {
    let temp = fixture()?;
    fs::write(temp.path().join("toolchain.json"), vec![b' '; 65_537])?;
    let error = failure(temp.path());
    assert_eq!(error.code(), "toolchain-read-failed");
    assert!(error.source().is_some());
    let file = temp.path().join("large");
    fs::write(&file, [0; 128])?;
    assert!(contract::sha256(&file, 127).is_err());
    assert!(contract::read_bounded(&file, 127).is_err());
    assert!(contract::sha256(&file, 128).is_ok());
    Ok(())
}

#[test]
fn safe_paths_reject_all_unsafe_components() -> Result<(), Box<dyn std::error::Error>> {
    let temp = fixture()?;
    let root = fs::canonicalize(temp.path())?;
    for relative in [
        "",
        "/tmp/file",
        "../file",
        "compiler/./lib.d.ts",
        "compiler//lib.d.ts",
        "C:/file",
        "compiler\\lib.d.ts",
    ] {
        assert!(
            contract::regular_file(&root, relative).is_err(),
            "{relative}"
        );
    }
    assert!(contract::regular_file(&root, "compiler/lib.d.ts").is_ok());
    Ok(())
}

#[test]
fn native_format_checks_reject_wrappers_and_wrong_architecture(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("executable");
    for (triple, magic, machine, offset) in [
        (
            "aarch64-apple-darwin",
            &[0xcf, 0xfa, 0xed, 0xfe][..],
            &[0x0c, 0, 0, 1][..],
            4,
        ),
        (
            "x86_64-apple-darwin",
            &[0xcf, 0xfa, 0xed, 0xfe][..],
            &[7, 0, 0, 1][..],
            4,
        ),
        (
            "aarch64-unknown-linux-gnu",
            &[0x7f, b'E', b'L', b'F', 2, 1][..],
            &[183, 0][..],
            18,
        ),
        (
            "x86_64-unknown-linux-gnu",
            &[0x7f, b'E', b'L', b'F', 2, 1][..],
            &[62, 0][..],
            18,
        ),
    ] {
        let target = contract::target(triple).ok_or("missing target")?;
        // Header-only fixtures validate parsing, never executable behavior.
        let mut bytes = [0_u8; 64];
        bytes[..magic.len()].copy_from_slice(magic);
        bytes[offset..offset + machine.len()].copy_from_slice(machine);
        fs::write(&path, bytes)?;
        assert!(contract::native_executable(&path, &target)?);
        bytes[offset] ^= 0xff;
        fs::write(&path, bytes)?;
        assert!(!contract::native_executable(&path, &target)?);
        fs::write(
            &path,
            b"#!/usr/bin/env node\nconsole.log('Version 7.0.2');\n",
        )?;
        assert!(!contract::native_executable(&path, &target)?);
    }
    for (triple, machine) in [
        ("aarch64-pc-windows-msvc", [0x64, 0xaa]),
        ("x86_64-pc-windows-msvc", [0x64, 0x86]),
    ] {
        let target = contract::target(triple).ok_or("missing target")?;
        let mut bytes = [0_u8; 80];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&64_u32.to_le_bytes());
        bytes[64..68].copy_from_slice(b"PE\0\0");
        bytes[68..70].copy_from_slice(&machine);
        fs::write(&path, bytes)?;
        assert!(contract::native_executable(&path, &target)?);
        bytes[68] ^= 0xff;
        fs::write(&path, bytes)?;
        assert!(!contract::native_executable(&path, &target)?);
    }
    Ok(())
}

#[test]
fn probe_errors_are_safe_inside_an_existing_runtime() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        // A genuine test executable is not a TypeScript compiler. This verifies
        // wrong-version failure and nested-runtime safety, not compilation.
        let error = probe::check(&std::env::current_exe()?)
            .err()
            .ok_or("test executable unexpectedly identified as compiler")?;
        assert_eq!(error.code(), "toolchain-compiler-failed");
        Ok::<(), Box<dyn std::error::Error>>(())
    })
}

#[cfg(unix)]
#[test]
fn rejects_file_directory_and_root_symlinks() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;
    let temp = fixture()?;
    let path = temp.path().join("compiler/lib.d.ts");
    fs::remove_file(&path)?;
    symlink("lib.dom.d.ts", path)?;
    assert_eq!(failure(temp.path()).code(), "toolchain-invalid");

    let temp = fixture()?;
    fs::rename(temp.path().join("compiler"), temp.path().join("elsewhere"))?;
    symlink("elsewhere", temp.path().join("compiler"))?;
    assert_eq!(failure(temp.path()).code(), "toolchain-invalid");
    let link = temp.path().join("root-link");
    symlink(temp.path(), &link)?;
    assert_eq!(failure(&link).code(), "toolchain-invalid");
    Ok(())
}
