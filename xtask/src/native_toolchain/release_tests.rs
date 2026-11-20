// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;

#[test]
fn requires_go_all_six_target_contracts_validate_without_executing_payloads() -> Result<(), String>
{
    // Synthetic headers/payloads prove portable inventory validation only.
    // They are deliberately not executable programs or source-provenance proof.
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    fs::write(temp.path().join("LICENSE"), b"preserve caller license")
        .map_err(|e| e.to_string())?;
    crate::native_toolchain::test_release_fixture(temp.path(), "3.1.2-hotfix.4")?;
    assert_eq!(
        fs::read(temp.path().join("LICENSE")).map_err(|e| e.to_string())?,
        b"preserve caller license"
    );
    crate::native_toolchain::validate_release(temp.path(), "3.1.2-hotfix.4")?;
    assert!(crate::native_toolchain::validate_release(temp.path(), "3.1.2").is_err());
    Ok(())
}

#[test]
fn complete_release_never_accepts_a_missing_target_sidecar_or_unknown_entry() -> Result<(), String>
{
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let directory = temp.path().join("publish/toolchains");
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    for target in TARGETS {
        assert!(contract::target(target).is_some());
        let name = filename("3.1.2", target);
        fs::write(directory.join(&name), b"checksum-failure fixture").map_err(|e| e.to_string())?;
        fs::write(directory.join(format!("{name}.sha256")), b"invalid")
            .map_err(|e| e.to_string())?;
    }
    let error = crate::native_toolchain::validate_release(temp.path(), "3.1.2")
        .err()
        .ok_or("corrupt distribution accepted")?;
    assert!(error.contains("SHA-256 mismatch"));
    let name = filename("3.1.2", TARGETS[0]);
    fs::remove_file(directory.join(format!("{name}.sha256"))).map_err(|e| e.to_string())?;
    let error = crate::native_toolchain::validate_release(temp.path(), "3.1.2")
        .err()
        .ok_or("missing sidecar accepted")?;
    assert!(error.contains("all six targets"));
    fs::write(directory.join("unknown"), b"unexpected").map_err(|e| e.to_string())?;
    assert!(crate::native_toolchain::validate_release(temp.path(), "3.1.2").is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn release_directory_symlinks_are_rejected() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let outside = tempfile::tempdir().map_err(|e| e.to_string())?;
    std::os::unix::fs::symlink(outside.path(), temp.path().join("publish"))
        .map_err(|e| e.to_string())?;
    assert!(crate::native_toolchain::validate_release(temp.path(), "3.1.2").is_err());
    Ok(())
}

#[test]
fn requires_go_portable_archive_streaming_guards() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let source = temp.path().join("validate.go");
    let tests = temp.path().join("validate_test.go");
    fs::write(&source, include_str!("validate.go")).map_err(|e| e.to_string())?;
    fs::write(&tests, include_str!("validate_test.go")).map_err(|e| e.to_string())?;
    let status = producer_go::command(temp.path())?
        .args(["test", "-p=2", "-count=1"])
        .arg(source)
        .arg(tests)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("portable archive tests failed ({status})"));
    }
    Ok(())
}

#[test]
fn requires_go_genuine_archive_is_validated_without_target_execution() -> Result<(), String> {
    let Some(archive) = std::env::var_os("WEBUI_TEST_TOOLCHAIN_ARCHIVE") else {
        eprintln!("genuine release archive test not exercised: set WEBUI_TEST_TOOLCHAIN_ARCHIVE");
        return Ok(());
    };
    let source = Path::new(&archive);
    let temporary = tempfile::tempdir().map_err(|e| e.to_string())?;
    let directory = fs::canonicalize(temporary.path()).map_err(|e| e.to_string())?;
    let name = filename("3.1.2", "aarch64-apple-darwin");
    fs::copy(source, directory.join(&name)).map_err(|e| e.to_string())?;
    fs::copy(
        source.with_file_name(format!("{name}.sha256")),
        directory.join(format!("{name}.sha256")),
    )
    .map_err(|e| e.to_string())?;
    // This controlled acceptance proves this real archive only. The public
    // complete-release API still refuses to accept a subset of the six targets.
    validate_archives(&directory, "3.1.2", &["aarch64-apple-darwin"])
}
