// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! WASM playground build task.
//!
//! Builds the `webui-wasm` crate to WebAssembly via `wasm-pack` and writes
//! handler, parser, and combined bundles into the documentation public assets.

use crate::util::{ensure_cargo_install, ensure_rustup_target};
use std::fs;
use std::path::Path;
use std::process::Command;

pub(crate) const WASM_OUTPUT_DIR: &str = "docs/.webui-press/public/wasm";

struct WasmVariant {
    label: &'static str,
    out_dir: &'static str,
    out_name: &'static str,
    features: &'static str,
}

const WASM_VARIANTS: &[WasmVariant] = &[
    WasmVariant {
        label: "all",
        out_dir: "all",
        out_name: "webui_wasm_all",
        features: "all",
    },
    WasmVariant {
        label: "handler",
        out_dir: "handler",
        out_name: "webui_wasm_handler",
        features: "handler",
    },
    WasmVariant {
        label: "parser",
        out_dir: "parser",
        out_name: "webui_wasm_parser",
        features: "parser",
    },
];

/// Build the webui-wasm packages for docs and release artifacts.
pub fn run() -> Result<(), String> {
    let crate_dir = Path::new("crates/webui-wasm");
    let out_dir = Path::new(WASM_OUTPUT_DIR);

    // 1. Ensure wasm-pack is installed (auto-install if missing)
    ensure_cargo_install("wasm-pack", "wasm-pack")?;

    // 2. Ensure wasm32-unknown-unknown target is installed
    ensure_rustup_target("wasm32-unknown-unknown")?;

    // 3. Clean output directory
    if out_dir.exists() {
        fs::remove_dir_all(out_dir)
            .map_err(|e| format!("Failed to clean {}: {}", out_dir.display(), e))?;
    }

    // 4. Run wasm-pack build for each feature set
    for variant in WASM_VARIANTS {
        build_variant(crate_dir, out_dir, variant)?;
    }

    Ok(())
}

fn build_variant(crate_dir: &Path, out_root: &Path, variant: &WasmVariant) -> Result<(), String> {
    let variant_out_dir = out_root.join(variant.out_dir);
    let out_dir_arg = format!("../../{}", variant_out_dir.display());

    let mut cmd = Command::new("wasm-pack");
    cmd.args([
        "build",
        &crate_dir.display().to_string(),
        "--target",
        "web",
        "--out-dir",
        &out_dir_arg,
        "--out-name",
        variant.out_name,
        "--no-default-features",
        "--features",
        variant.features,
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
        return Err(format!("wasm-pack failed for {}:\n{msg}", variant.label));
    }

    // 5. Remove wasm-pack generated .gitignore
    let gitignore = variant_out_dir.join(".gitignore");
    if gitignore.exists() {
        let _ = fs::remove_file(&gitignore);
    }

    // 6. Report
    let wasm_path = variant_out_dir.join(format!("{}_bg.wasm", variant.out_name));
    let meta = fs::metadata(&wasm_path)
        .map_err(|e| format!("missing expected WASM output {}: {e}", wasm_path.display()))?;
    let size_kb = meta.len() / 1024;
    eprintln!(
        "  {} [{}] Output: {}",
        console::style("✔").green(),
        console::style(variant.label).bold(),
        console::style(variant_out_dir.display()).bold()
    );
    eprintln!(
        "  {} [{}] Size:   {}",
        console::style("✔").green(),
        console::style(variant.label).bold(),
        console::style(format!("{} KB", size_kb)).bold()
    );

    Ok(())
}
