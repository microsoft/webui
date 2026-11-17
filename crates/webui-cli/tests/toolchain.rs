// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Producer/consumer acceptance using explicit, genuine native local artifacts.
//! Opt in with WEBUI_TEST_NATIVE_CHECKER_DIR pointing at the pinned upstream
//! platform package root. This is development verification, not acquisition.

// This cross-boundary test intentionally compiles producer and consumer modules
// independently; both share the same source-owned archive format contract.
#![allow(clippy::duplicate_mod)]

#[path = "../../../xtask/src/native_toolchain.rs"]
mod native_toolchain;
#[path = "../src/commands/toolchain.rs"]
mod toolchain;

use std::fs;
use std::path::Path;
use std::process::Command;

#[test]
fn resolver_child() -> Result<(), Box<dyn std::error::Error>> {
    let Some(mode) = std::env::var_os("WEBUI_TOOLCHAIN_ACCEPTANCE_CHILD") else {
        return Ok(());
    };
    if mode == "invalid" {
        let error = toolchain::resolve()
            .err()
            .ok_or("invalid override unexpectedly passed")?;
        assert!(matches!(
            error.code(),
            "toolchain-invalid" | "toolchain-read-failed"
        ));
        return Ok(());
    }
    if mode == "bad-compiler" {
        let error = toolchain::resolve()
            .err()
            .ok_or("wrong native compiler unexpectedly passed")?;
        assert_eq!(error.code(), "toolchain-compiler-failed");
        return Ok(());
    }
    let tooling = toolchain::resolve()?;
    assert!(tooling.compiler.is_absolute());
    let project =
        std::env::var_os("WEBUI_TOOLCHAIN_ACCEPTANCE_PROJECT").ok_or("missing project")?;
    let output = Command::new(&tooling.compiler)
        .current_dir(project)
        .args([
            "--noEmit",
            "--strict",
            "--target",
            "ES2022",
            "--lib",
            "ES2022,DOM",
            "main.ts",
        ])
        .output()?;
    let diagnostics = String::from_utf8_lossy(&output.stdout);
    if mode == "bad-source" {
        assert!(!output.status.success());
        assert!(diagnostics.contains("TS2322"), "{diagnostics}");
    } else {
        assert!(
            output.status.success(),
            "{diagnostics}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn child(root: &Path, project: &Path, mode: &str) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new(std::env::current_exe()?)
        .args(["--exact", "resolver_child", "--nocapture"])
        .env("WEBUI_TOOLCHAIN_ACCEPTANCE_CHILD", mode)
        .env("WEBUI_TOOLCHAIN_ACCEPTANCE_PROJECT", project)
        .env("WEBUI_TOOLCHAIN_DIR", root)
        // Neither the resolver nor the actual checker can find Node/npm/tsc.
        .env("PATH", "")
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[test]
fn genuine_native_archive_checks_types_without_node() -> Result<(), Box<dyn std::error::Error>> {
    let Some(checker) = std::env::var_os("WEBUI_TEST_NATIVE_CHECKER_DIR") else {
        eprintln!("native artifact acceptance not exercised: set WEBUI_TEST_NATIVE_CHECKER_DIR explicitly");
        return Ok(());
    };
    let temp = tempfile::tempdir()?;
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // Staging stays inside this test's output root, with the actual WebUI notice.
    fs::copy(workspace.join("LICENSE"), temp.path().join("LICENSE"))?;
    let options = native_toolchain::StageOptions {
        root: temp.path(),
        version: env!("CARGO_PKG_VERSION"),
        target: env!("WEBUI_NATIVE_TARGET"),
        checker_dir: Path::new(&checker),
        cli: Path::new(env!("CARGO_BIN_EXE_webui")),
    };
    let staged = native_toolchain::stage(&options)?;
    assert!(staged.archive.is_file());
    assert!(staged.checksum.is_file());
    let archive_name = staged
        .archive
        .file_name()
        .ok_or("missing name")?
        .to_string_lossy();
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut hash = Sha256::new();
    let mut file = fs::File::open(&staged.archive)?;
    let mut buffer = vec![0_u8; 16 * 1024];
    loop {
        let size = file.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        hash.update(&buffer[..size]);
    }
    use std::fmt::Write;
    let mut digest = String::with_capacity(64);
    for byte in hash.finalize() {
        write!(&mut digest, "{byte:02x}")?;
    }
    assert_eq!(
        fs::read_to_string(&staged.checksum)?,
        format!("{digest}  {archive_name}\n")
    );
    let extracted = temp.path().join("relocated toolchain");
    let project = temp.path().join("project");
    fs::create_dir(&extracted)?;
    fs::create_dir(&project)?;
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(&staged.archive)
        .arg("-C")
        .arg(&extracted)
        .status()?;
    assert!(status.success());
    fs::write(project.join("main.ts"), "const count: number = 1;\ndocument.title = new Map<string, number>([['count', count]]).size.toString();\n")?;
    child(&extracted, &project, "good-source")?;
    fs::write(
        project.join("main.ts"),
        "const count: number = 'deliberate type error';\n",
    )?;
    child(&extracted, &project, "bad-source")?;
    assert!(!project.join("main.js").exists());
    child(Path::new(""), &project, "invalid")?;
    child(&temp.path().join("missing"), &project, "invalid")?;
    // A genuine native executable of the right architecture is still not the
    // pinned checker. Recompute just its hash to reach the version guard.
    let compiler_name = if cfg!(windows) { "tsc.exe" } else { "tsc" };
    let compiler_path = extracted.join("compiler").join(compiler_name);
    fs::copy(env!("CARGO_BIN_EXE_webui"), &compiler_path)?;
    let metadata_path = extracted.join("toolchain.json");
    let mut metadata: serde_json::Value = serde_json::from_slice(&fs::read(&metadata_path)?)?;
    let mut hash = Sha256::new();
    let mut file = fs::File::open(&compiler_path)?;
    loop {
        let size = file.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        hash.update(&buffer[..size]);
    }
    let mut digest = String::with_capacity(64);
    for byte in hash.finalize() {
        write!(&mut digest, "{byte:02x}")?;
    }
    metadata["files"][format!("compiler/{compiler_name}")] = serde_json::Value::from(digest);
    fs::write(&metadata_path, serde_json::to_vec(&metadata)?)?;
    child(&extracted, &project, "bad-compiler")?;
    fs::write(
        extracted.join("compiler/lib.dom.d.ts"),
        b"corrupted after extraction",
    )?;
    child(&extracted, &project, "invalid")?;
    Ok(())
}
