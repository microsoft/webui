// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! WASM playground build task.
//!
//! Builds the `webui-wasm` crate to WebAssembly via `wasm-pack` and patches
//! the generated JS glue for browser compatibility.

use crate::util::{ensure_cargo_install, ensure_rustup_target};
use std::fs;
use std::path::Path;
use std::process::Command;

/// Build the webui-wasm package for the playground.
pub fn run() -> Result<(), String> {
    let crate_dir = Path::new("crates/webui-wasm");
    let out_dir = Path::new("docs/.webui-press/public/wasm");

    // 1. Ensure wasm-pack is installed (auto-install if missing)
    ensure_cargo_install("wasm-pack", "wasm-pack")?;

    // 2. Ensure wasm32-unknown-unknown target is installed
    ensure_rustup_target("wasm32-unknown-unknown")?;

    // 3. Clean output directory
    if out_dir.exists() {
        fs::remove_dir_all(out_dir)
            .map_err(|e| format!("Failed to clean {}: {}", out_dir.display(), e))?;
    }

    // 4. Run wasm-pack build
    let mut cmd = Command::new("wasm-pack");
    cmd.args([
        "build",
        &crate_dir.display().to_string(),
        "--target",
        "web",
        "--out-dir",
        &format!("../../{}", out_dir.display()),
    ]);

    // Suppress wasm-pack's verbose output — show only on failure
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let output = cmd
        .output()
        .map_err(|e| format!("wasm-pack failed to start: {}", e))?;
    if !output.status.success() {
        let mut msg = String::new();
        if let Ok(s) = String::from_utf8(output.stdout) {
            msg.push_str(&s);
        }
        if let Ok(s) = String::from_utf8(output.stderr) {
            msg.push_str(&s);
        }
        return Err(format!("wasm-pack failed:\n{msg}"));
    }

    // 5. Remove wasm-pack generated .gitignore
    let gitignore = out_dir.join(".gitignore");
    if gitignore.exists() {
        let _ = fs::remove_file(&gitignore);
    }

    // 6. Report
    let wasm_path = out_dir.join("webui_wasm_bg.wasm");
    if let Ok(meta) = fs::metadata(&wasm_path) {
        let size_kb = meta.len() / 1024;
        eprintln!(
            "  {} Output: {}",
            console::style("✔").green(),
            console::style(out_dir.display()).bold()
        );
        eprintln!(
            "  {} Size:   {}",
            console::style("✔").green(),
            console::style(format!("{} KB", size_kb)).bold()
        );
    }

    Ok(())
}
