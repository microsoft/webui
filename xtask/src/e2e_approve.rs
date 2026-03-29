// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Download CI screenshot baselines and apply them locally.
//!
//! Usage: `cargo xtask e2e-approve [run-id]`
//!
//! Downloads the `e2e-updated-baselines` artifact from the latest CI run on the
//! current branch (or a specific run if `run-id` is provided), extracts
//! the PNGs, and copies them into the correct snapshot directories.
//!
//! Requires the `gh` CLI to be installed and authenticated.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use crate::util;

pub fn run(run_id: Option<&str>) -> ExitCode {
    eprintln!(
        "\n{} E2E approve baselines",
        console::style("▸").cyan().bold(),
    );

    // Ensure gh CLI is available
    if !util::which_exists("gh") {
        eprintln!(
            "  {} {} is required but not installed",
            console::style("✘").red().bold(),
            console::style("gh").bold(),
        );
        eprintln!(
            "    {} Install from {}",
            console::style("▸").cyan(),
            console::style("https://cli.github.com").bold(),
        );
        return ExitCode::FAILURE;
    }

    // Resolve run ID: use provided ID or find the latest on current branch
    let resolved_run_id = match run_id {
        Some(id) => id.to_string(),
        None => match find_latest_run() {
            Ok(id) => id,
            Err(msg) => {
                eprintln!("  {} {msg}", console::style("✘").red().bold());
                return ExitCode::FAILURE;
            }
        },
    };

    eprintln!(
        "  {} Run {}",
        console::style("▸").dim(),
        console::style(&resolved_run_id).bold(),
    );

    // Create temp directory for download
    let tmp_dir = match std::env::temp_dir()
        .join(format!("webui-e2e-approve-{resolved_run_id}"))
        .to_str()
        .map(|s| s.to_string())
    {
        Some(p) => p,
        None => {
            eprintln!(
                "  {} Failed to create temp path",
                console::style("✘").red().bold(),
            );
            return ExitCode::FAILURE;
        }
    };

    // Clean up any previous download
    let _ = std::fs::remove_dir_all(&tmp_dir);

    // Download the artifact
    eprintln!("  {} Downloading snapshots...", console::style("▸").dim(),);

    let status = Command::new("gh")
        .args([
            "run",
            "download",
            &resolved_run_id,
            "--name",
            "e2e-updated-baselines",
            "--dir",
            &tmp_dir,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(_) => {
            eprintln!(
                "  {} Failed to download e2e-updated-baselines artifact",
                console::style("✘").red().bold(),
            );
            eprintln!(
                "    {} The run may not have an e2e-updated-baselines artifact. \
                 Check: {}",
                console::style("hint:").yellow(),
                console::style(format!("gh run view {resolved_run_id} --json jobs")).dim(),
            );
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!(
                "  {} Failed to run gh: {e}",
                console::style("✘").red().bold(),
            );
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return ExitCode::FAILURE;
        }
    }

    // Copy snapshots to the right places
    let tmp_path = PathBuf::from(&tmp_dir);
    let mut copied = 0u32;

    // Walk the downloaded directory and copy PNGs to the workspace
    if let Err(msg) = copy_snapshots(&tmp_path, &mut copied) {
        eprintln!("  {} {msg}", console::style("✘").red().bold());
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return ExitCode::FAILURE;
    }

    // Clean up
    let _ = std::fs::remove_dir_all(&tmp_dir);

    if copied == 0 {
        eprintln!(
            "  {} No snapshot files found in artifact",
            console::style("⚠").yellow(),
        );
        return ExitCode::FAILURE;
    }

    eprintln!(
        "\n{} Applied {} baseline(s) from CI run {}",
        console::style("✔").green(),
        console::style(copied).bold(),
        console::style(&resolved_run_id).dim(),
    );
    eprintln!(
        "  {} Review with {} then commit",
        console::style("hint:").yellow(),
        console::style("git diff --stat").bold(),
    );

    ExitCode::SUCCESS
}

/// Find the latest workflow run on the current branch.
fn find_latest_run() -> Result<String, String> {
    // Get current branch
    let branch_output = Command::new("git")
        .args(["branch", "--show-current"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("Failed to get current branch: {e}"))?;

    let branch = String::from_utf8_lossy(&branch_output.stdout)
        .trim()
        .to_string();

    if branch.is_empty() {
        return Err("Could not determine current branch (detached HEAD?)".into());
    }

    eprintln!(
        "  {} Branch {}",
        console::style("▸").dim(),
        console::style(&branch).bold(),
    );

    // Find latest run on this branch
    let output = Command::new("gh")
        .args([
            "run",
            "list",
            "--branch",
            &branch,
            "--limit",
            "1",
            "--json",
            "databaseId",
            "--jq",
            ".[0].databaseId",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("Failed to query GitHub runs: {e}"))?;

    let run_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if run_id.is_empty() || run_id == "null" {
        return Err(format!(
            "No CI runs found on branch '{branch}'. Push first, then retry."
        ));
    }

    Ok(run_id)
}

/// Iteratively copy snapshot PNGs from the downloaded artifact into the workspace.
///
/// Uses an explicit stack instead of recursion. The `upload-artifact@v4` action
/// preserves the full directory structure, so the relative path within the
/// download is already the correct workspace-relative destination.
fn copy_snapshots(download_root: &Path, count: &mut u32) -> Result<(), String> {
    let mut stack = vec![download_root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|e| format!("Failed to read {}: {e}", dir.display()))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read entry: {e}"))?;
            let path = entry.path();

            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("png") {
                // Artifact preserves workspace-relative paths, e.g.:
                //   <download_root>/examples/app/<app>/tests/<spec>-snapshots/<file>.png
                //   <download_root>/packages/<pkg>/tests/<spec>-snapshots/<file>.png
                let relative = path.strip_prefix(download_root).map_err(|_| {
                    format!(
                        "Unexpected artifact path: {} is not under {}",
                        path.display(),
                        download_root.display(),
                    )
                })?;
                let dest = relative.to_path_buf();

                if let Some(parent) = dest.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }

                std::fs::copy(&path, &dest).map_err(|e| {
                    format!(
                        "Failed to copy {} → {}: {e}",
                        path.display(),
                        dest.display()
                    )
                })?;

                eprintln!(
                    "  {} {}",
                    console::style("✔").green(),
                    console::style(dest.display()).dim(),
                );
                *count += 1;
            }
        }
    }

    Ok(())
}
