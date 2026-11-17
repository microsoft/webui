// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;

#[test]
fn frozen_lock_pins_each_target_independently_of_runner_architecture() -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut hashes = BTreeSet::new();
    for triple in [
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "aarch64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
        "aarch64-pc-windows-msvc",
        "x86_64-pc-windows-msvc",
    ] {
        let target = contract::target(triple).ok_or("missing target")?;
        let locked = locked_checker(&root, &target)?;
        assert_eq!(locked.digest.len(), 64);
        assert!(locked.package.ends_with("@7.0.2"));
        assert!(locked
            .url
            .starts_with("https://github.com/microsoft/TypeScript/releases/download/v7.0.2/"));
        assert!(locked
            .url
            .contains(&format!("typescript-{}-{}", target.os, target.cpu)));
        assert!(hashes.insert(locked.integrity));
    }
    Ok(())
}

#[test]
fn rejects_missing_wrong_algorithm_and_malformed_integrity() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let target = contract::target("aarch64-apple-darwin").ok_or("missing target")?;
    for integrity in ["sha256-bad", "sha512-bad", "sha512-AA==", ""] {
        let lockfile = format!(
            "packages:\n  '@typescript/typescript-darwin-arm64@7.0.2':\n    resolution:\n      integrity: '{integrity}'\n"
        );
        fs::write(temp.path().join("pnpm-lock.yaml"), lockfile).map_err(|e| e.to_string())?;
        assert!(locked_checker(temp.path(), &target).is_err());
    }
    fs::write(temp.path().join("pnpm-lock.yaml"), "packages: {}\n").map_err(|e| e.to_string())?;
    assert!(locked_checker(temp.path(), &target).is_err());
    Ok(())
}

#[test]
fn multi_document_lock_rejects_ambiguous_source_identity() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let target = contract::target("aarch64-apple-darwin").ok_or("missing target")?;
    let document = format!("---\npackages:\n  '@typescript/typescript-darwin-arm64@7.0.2':\n    resolution:\n      integrity: 'sha512-{}'\n", STANDARD.encode([0; 64]));
    fs::write(
        temp.path().join("pnpm-lock.yaml"),
        format!("{document}{document}"),
    )
    .map_err(|e| e.to_string())?;
    let error = locked_checker(temp.path(), &target)
        .err()
        .ok_or("duplicate identity passed")?;
    assert!(error.contains("duplicate frozen resolution"));
    Ok(())
}

#[test]
fn checksum_failure_precedes_archive_interpretation() -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let package = temp.path().join("invalid.tgz");
    fs::write(&package, b"not a gzip or tar archive").map_err(|e| e.to_string())?;
    let output = temp.path().join("source");
    fs::create_dir(&output).map_err(|e| e.to_string())?;
    let error = prepare(&SourceOptions {
        root: &root,
        target: "aarch64-apple-darwin",
        package: Some(&package),
        checker_dir: None,
        temporary: &output,
    })
    .err()
    .ok_or("corrupt package passed")?;
    assert!(error.contains("SHA-512 mismatch"));
    assert!(!output.join("checker").exists());
    Ok(())
}

#[test]
fn streaming_integrity_and_directory_comparison_detect_modifications() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let package = temp.path().join("package.tgz");
    fs::write(&package, b"integrity-only synthetic fixture").map_err(|e| e.to_string())?;
    let digest = Sha512::digest(b"integrity-only synthetic fixture");
    verify(&package, digest.as_slice())?;
    assert!(verify(&package, &[0; 64]).is_err());
    let local = temp.path().join("local");
    let verified = temp.path().join("verified");
    fs::create_dir(&local).map_err(|e| e.to_string())?;
    fs::create_dir(&verified).map_err(|e| e.to_string())?;
    for directory in [&local, &verified] {
        fs::write(directory.join("LICENSE"), b"fixture notice").map_err(|e| e.to_string())?;
    }
    let files = BTreeSet::from(["LICENSE".to_owned()]);
    compare_directory(&local, &verified, &files)?;
    fs::write(local.join("LICENSE"), b"modified").map_err(|e| e.to_string())?;
    assert!(compare_directory(&local, &verified, &files).is_err());
    Ok(())
}

#[test]
fn explicit_genuine_package_can_be_sourced_without_execution() -> Result<(), String> {
    let Some(package) = std::env::var_os("WEBUI_TEST_TYPESCRIPT_PACKAGE") else {
        eprintln!("genuine upstream sourcing not exercised: set WEBUI_TEST_TYPESCRIPT_PACKAGE and WEBUI_TEST_TYPESCRIPT_TARGET");
        return Ok(());
    };
    let triple = std::env::var("WEBUI_TEST_TYPESCRIPT_TARGET").map_err(|e| e.to_string())?;
    let checker_dir = std::env::var_os("WEBUI_TEST_TYPESCRIPT_DIRECTORY");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let temporary = tempfile::tempdir().map_err(|e| e.to_string())?;
    let source = prepare(&SourceOptions {
        root: &root,
        target: &triple,
        package: Some(Path::new(&package)),
        checker_dir: checker_dir.as_deref().map(Path::new),
        temporary: temporary.path(),
    })?;
    let target = contract::target(&triple).ok_or("unsupported test target")?;
    assert!(contract::native_executable(
        &source.directory.join("lib").join(target.compiler),
        &target
    )
    .map_err(|e| e.to_string())?);
    assert!(
        fs::metadata(source.directory.join("lib/lib.dom.d.ts"))
            .map_err(|e| e.to_string())?
            .len()
            > 0
    );
    Ok(())
}
