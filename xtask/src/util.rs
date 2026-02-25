//! Shared utilities for xtask commands.

use console::Style;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// ── Styled output (matches webui-cli output.rs) ─────────────────────────

pub struct Printer {
    pub cyan: Style,
    pub green: Style,
    pub red: Style,
    pub yellow: Style,
    pub dim: Style,
    pub bold: Style,
}

impl Printer {
    pub fn new() -> Self {
        Self {
            cyan: Style::new().cyan().bold(),
            green: Style::new().green(),
            red: Style::new().red().bold(),
            yellow: Style::new().yellow(),
            dim: Style::new().dim(),
            bold: Style::new().bold(),
        }
    }
}

/// Run a command and return Ok if it exits with status 0.
pub fn run_command(cmd: &str, args: &[&str], cwd: Option<&Path>) -> Result<(), String> {
    let mut command = Command::new(cmd);
    command.args(args);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }

    match command.status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("exit code {}", status.code().unwrap_or(1))),
        Err(error) => Err(error.to_string()),
    }
}

/// Collect immediate child directories of `root`, sorted.
pub fn collect_child_dirs(root: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = fs::read_dir(root).map_err(|error| error.to_string())?;
    let mut dirs = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }

    dirs.sort();
    Ok(dirs)
}

/// Extract the last path component as a display name.
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
        .unwrap_or_else(|| path.display().to_string())
}

/// Check if a command exists on PATH.
pub fn which_exists(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}
