// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use serde_json::Value;

#[test]
fn generated_rust_server_owns_its_workspace_inside_a_nonmember_app() -> Result<()> {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    fs::create_dir_all(&fixtures)?;
    let fixture = tempfile::tempdir_in(fixtures)?;
    let root = fixture.path().canonicalize()?;
    let workspace = "[workspace]\nmembers = []\n";
    fs::write(root.join("Cargo.toml"), workspace)?;
    let app = root.join("nested-app");
    let initialized = Command::new(env!("CARGO_BIN_EXE_webui"))
        .args(["--format=json", "init"])
        .arg(&app)
        .args(["--server", "rust", "--no-interactive"])
        .stdin(Stdio::null())
        .output()
        .context("running webui init")?;
    anyhow::ensure!(
        initialized.status.success(),
        "init failed: {} {}",
        String::from_utf8_lossy(&initialized.stdout),
        String::from_utf8_lossy(&initialized.stderr)
    );
    let manifest = app.join("server/Cargo.toml");
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .arg("--manifest-path")
        .arg(&manifest)
        .current_dir(&app)
        .stdin(Stdio::null())
        .output()
        .context("running generated server Cargo metadata")?;
    anyhow::ensure!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout)?;
    let workspace_root = metadata["workspace_root"]
        .as_str()
        .context("Cargo workspace root")?;
    assert_eq!(
        Path::new(workspace_root).canonicalize()?,
        app.join("server").canonicalize()?
    );
    let package_manifest = metadata["packages"][0]["manifest_path"]
        .as_str()
        .context("generated package manifest")?;
    assert_eq!(
        Path::new(package_manifest).canonicalize()?,
        manifest.canonicalize()?
    );
    assert_eq!(fs::read_to_string(root.join("Cargo.toml"))?, workspace);
    Ok(())
}
