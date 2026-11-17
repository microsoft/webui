// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;

// These tiny, synthetic package fixtures only prove rejection behavior.
fn source(target: &contract::Target) -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir())?)?;
    fs::create_dir(temp.path().join("lib"))?;
    let metadata = Value::Object(Map::from_iter([
        (
            "name".to_owned(),
            Value::from(format!(
                "@typescript/typescript-{}-{}",
                target.os, target.cpu
            )),
        ),
        ("version".to_owned(), Value::from("7.0.2")),
        ("os".to_owned(), Value::Array(vec![Value::from(target.os)])),
        (
            "cpu".to_owned(),
            Value::Array(vec![Value::from(target.cpu)]),
        ),
    ]));
    fs::write(
        temp.path().join("package.json"),
        serde_json::to_vec(&metadata)?,
    )?;
    for name in contract::libraries() {
        fs::write(
            temp.path().join("lib").join(name),
            b"synthetic rejection fixture",
        )?;
    }
    Ok(temp)
}

#[test]
fn rejects_mismatched_upstream_identity_and_incomplete_libs(
) -> Result<(), Box<dyn std::error::Error>> {
    let target = contract::target("aarch64-apple-darwin").ok_or("missing target")?;
    let temp = source(&target)?;
    validate_package(temp.path(), &target)?;
    let path = temp.path().join("package.json");
    let mut metadata: Value = serde_json::from_slice(&fs::read(&path)?)?;
    metadata["version"] = Value::from("3.1.2");
    fs::write(&path, serde_json::to_vec(&metadata)?)?;
    assert!(validate_package(temp.path(), &target).is_err());
    let temp = source(&target)?;
    fs::remove_file(temp.path().join("lib/lib.dom.d.ts"))?;
    assert!(validate_package(temp.path(), &target).is_err());
    let temp = source(&target)?;
    fs::write(temp.path().join("lib/lib.unknown.d.ts"), b"unexpected")?;
    assert!(validate_package(temp.path(), &target).is_err());
    Ok(())
}

#[test]
fn rejects_script_compilers_even_with_matching_package_identity(
) -> Result<(), Box<dyn std::error::Error>> {
    let triple = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => return Err("unsupported native test host".into()),
    };
    let target = contract::target(triple).ok_or("missing target")?;
    let temp = source(&target)?;
    fs::write(
        temp.path().join("lib").join(target.compiler),
        b"#!/usr/bin/env node\nconsole.log('Version 7.0.2');\n",
    )?;
    let cli = std::env::current_exe()?;
    let options = StageOptions {
        root: temp.path(),
        version: "3.1.2",
        target: triple,
        checker_dir: temp.path(),
        cli: &cli,
    };
    let error = stage(&options)
        .err()
        .ok_or("script compiler was accepted")?;
    assert!(error.contains("native binary"));
    assert!(error.contains(target.compiler));
    assert!(!temp.path().join("publish").exists());
    Ok(())
}

#[test]
fn release_names_cannot_escape_and_unsupported_targets_fail(
) -> Result<(), Box<dyn std::error::Error>> {
    for version in ["", "../escape", "/absolute", "3.1.2/other", "3.1.2\\other"] {
        assert!(validate_version(version).is_err());
    }
    for version in ["3.1.2", "3.1.2-hotfix.1"] {
        validate_version(version)?;
    }
    let temp = tempfile::tempdir()?;
    let options = StageOptions {
        root: temp.path(),
        version: "3.1.2",
        target: "x86_64-unknown-linux-musl",
        checker_dir: temp.path(),
        cli: temp.path(),
    };
    assert!(stage(&options)
        .err()
        .ok_or("unexpected success")?
        .contains("unsupported"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn rejects_symlink_sources_and_output_directories() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;
    let target = contract::target("aarch64-apple-darwin").ok_or("missing target")?;
    let temp = source(&target)?;
    let library = temp.path().join("lib/lib.d.ts");
    fs::remove_file(&library)?;
    symlink("lib.dom.d.ts", library)?;
    assert!(validate_package(temp.path(), &target).is_err());
    let link = temp.path().join("source-link");
    symlink(temp.path(), &link)?;
    assert!(canonical_directory(&link).is_err());
    assert!(regular_input(&link).is_err());
    let outside = tempfile::tempdir()?;
    symlink(outside.path(), temp.path().join("publish"))?;
    assert!(output_directory(temp.path()).is_err());
    assert!(fs::read_dir(outside.path())?.next().is_none());
    Ok(())
}

#[test]
fn cross_target_staging_never_executes_inputs() -> Result<(), Box<dyn std::error::Error>> {
    // Deliberately non-executable, synthetic foreign-target headers prove only
    // staging policy, not genuine native compilation or release validity.
    let (triple, magic, cpu, offset) = if cfg!(target_os = "linux") {
        (
            "aarch64-apple-darwin",
            &[0xcf, 0xfa, 0xed, 0xfe][..],
            &[0x0c, 0, 0, 1][..],
            4,
        )
    } else {
        (
            "x86_64-unknown-linux-gnu",
            &[0x7f, b'E', b'L', b'F', 2, 1][..],
            &[62, 0][..],
            18,
        )
    };
    let target = contract::target(triple).ok_or("missing target")?;
    let temp = source(&target)?;
    let mut header = [0_u8; 64];
    header[..magic.len()].copy_from_slice(magic);
    header[offset..offset + cpu.len()].copy_from_slice(cpu);
    let cli = temp.path().join("cli");
    fs::write(&cli, header)?;
    fs::write(temp.path().join("lib").join(target.compiler), header)?;
    fs::write(temp.path().join("LICENSE"), b"synthetic notice")?;
    fs::write(temp.path().join("NOTICE.txt"), b"synthetic notice")?;
    let staged = stage(&StageOptions {
        root: temp.path(),
        version: "3.1.2",
        target: triple,
        checker_dir: temp.path(),
        cli: &cli,
    })?;
    assert!(staged.archive.is_file());
    assert!(staged.checksum.is_file());
    Ok(())
}
