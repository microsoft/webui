// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Shared utilities for xtask commands.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Return the workspace root directory.
///
/// This is resolved from the `xtask` crate directory at compile time, so it
/// remains correct regardless of where `cargo xtask ...` is invoked from.
pub fn workspace_root() -> Result<PathBuf, String> {
    let xtask_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let Some(root) = xtask_dir.parent() else {
        return Err(format!(
            "Failed to resolve workspace root from {}",
            xtask_dir.display()
        ));
    };
    Ok(root.to_path_buf())
}

/// Run a command and return Ok if it exits with status 0.
pub fn run_command(cmd: &str, args: &[&str], cwd: Option<&Path>) -> Result<(), String> {
    let mut command = build_command(cmd, args);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }

    match command.status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("exit code {}", status.code().unwrap_or(1))),
        Err(error) => Err(error.to_string()),
    }
}

/// Run a command with captured output. Returns Ok on success, Err with the
/// combined stdout+stderr on failure.
pub fn run_command_quiet(cmd: &str, args: &[&str], cwd: Option<&Path>) -> Result<(), String> {
    use std::process::Stdio;

    let mut command = build_command(cmd, args);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }

    match command.output() {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => {
            let mut msg = String::new();
            if let Ok(s) = String::from_utf8(output.stdout) {
                msg.push_str(&s);
            }
            if let Ok(s) = String::from_utf8(output.stderr) {
                msg.push_str(&s);
            }
            if msg.is_empty() {
                msg = format!("exit code {}", output.status.code().unwrap_or(1));
            }
            Err(msg)
        }
        Err(error) => Err(error.to_string()),
    }
}

/// Build a [`Command`] for `cmd` with `args`, resolving `.cmd`/`.bat` scripts
/// on Windows.
///
/// On Windows `CreateProcessW` cannot launch `.cmd`/`.bat` scripts directly.
/// This function uses `which` to resolve the executable path and, when the
/// target is a shell script, wraps it in `cmd.exe /c <resolved_path>`.
pub fn build_command(cmd: &str, args: &[&str]) -> Command {
    #[cfg(windows)]
    {
        resolve_windows_command(cmd, args)
    }
    #[cfg(not(windows))]
    {
        let mut c = Command::new(cmd);
        c.args(args);
        c
    }
}

#[cfg(windows)]
fn resolve_windows_command(cmd: &str, args: &[&str]) -> Command {
    if let Ok(resolved) = which::which(cmd) {
        let ext = resolved
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());

        if matches!(ext.as_deref(), Some("cmd" | "bat")) {
            let mut c = Command::new("cmd");
            c.arg("/c").arg(&resolved).args(args);
            return c;
        }

        let mut c = Command::new(&resolved);
        c.args(args);
        return c;
    }

    // Fallback: let cmd.exe attempt resolution.
    let mut c = Command::new("cmd");
    c.arg("/c").arg(cmd).args(args);
    c
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

/// Ensure a tool installed via `cargo install` is available, installing it
/// automatically if missing.
///
/// `crate_name` is the crate to install (e.g. `"cargo-deny"`, `"wasm-pack"`).
/// `binary` is the executable name to probe on PATH (e.g. `"cargo-deny"`,
/// `"wasm-pack"`).
pub fn ensure_cargo_install(crate_name: &str, binary: &str) -> Result<(), String> {
    if which_exists(binary) {
        return Ok(());
    }

    eprintln!(
        "    {} not found — installing…",
        console::style(crate_name).yellow()
    );
    run_command("cargo", &["install", crate_name], None)
        .map_err(|e| format!("failed to install {crate_name}: {e}"))
}

/// Ensure a rustup component (e.g. `clippy`, `rustfmt`) is available,
/// adding it automatically if missing.
pub fn ensure_rustup_component(component: &str) -> Result<(), String> {
    let output = Command::new("rustup")
        .args(["component", "list", "--installed"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("failed to run rustup: {e}"))?;

    let installed = String::from_utf8_lossy(&output.stdout);
    if installed.lines().any(|line| line.starts_with(component)) {
        return Ok(());
    }

    eprintln!(
        "    rustup component '{}' not found — adding…",
        console::style(component).yellow()
    );
    run_command("rustup", &["component", "add", component], None)
        .map_err(|e| format!("failed to add rustup component {component}: {e}"))
}

/// Ensure a rustup target (e.g. `wasm32-unknown-unknown`) is installed,
/// adding it automatically if missing.
pub fn ensure_rustup_target(target: &str) -> Result<(), String> {
    let output = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("failed to run rustup: {e}"))?;

    let installed = String::from_utf8_lossy(&output.stdout);
    if installed.lines().any(|line| line == target) {
        return Ok(());
    }

    eprintln!(
        "    rustup target '{}' not found — adding…",
        console::style(target).yellow()
    );
    run_command("rustup", &["target", "add", target], None)
        .map_err(|e| format!("failed to add rustup target {target}: {e}"))
}

/// Ensure Docker is installed and the daemon is running.
///
/// Prints friendly, platform-specific installation instructions if Docker
/// is not found, then returns an error. Follows the same pattern as
/// [`ensure_cargo_install`] and [`ensure_rustup_component`].
pub fn ensure_docker() -> Result<(), String> {
    // Check CLI availability
    let cli_ok = Command::new("docker")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !cli_ok {
        eprintln!(
            "\n  {} Docker is not installed",
            console::style("✘").red().bold(),
        );
        eprintln!();
        print_docker_install_hint();
        eprintln!();
        eprintln!(
            "    {} Use {} to skip Docker and run tests directly on the host.",
            console::style("hint:").yellow(),
            console::style("cargo xtask e2e --no-docker").bold(),
        );
        return Err("Docker is not installed — see instructions above".into());
    }

    // Check daemon is running
    let daemon_ok = Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !daemon_ok {
        eprintln!(
            "\n  {} Docker daemon is not running",
            console::style("✘").red().bold(),
        );
        eprintln!();
        print_docker_start_hint();
        eprintln!();
        eprintln!(
            "    {} Use {} to skip Docker and run tests directly on the host.",
            console::style("hint:").yellow(),
            console::style("cargo xtask e2e --no-docker").bold(),
        );
        return Err("Docker daemon is not running — see instructions above".into());
    }

    Ok(())
}

/// Print platform-specific Docker installation instructions.
fn print_docker_install_hint() {
    #[cfg(target_os = "macos")]
    {
        eprintln!("    Install Docker Desktop for macOS:");
        eprintln!(
            "      {} {}",
            console::style("▸").cyan(),
            console::style("brew install --cask docker").bold(),
        );
        eprintln!(
            "      {} or download from {}",
            console::style("▸").cyan(),
            console::style("https://docs.docker.com/desktop/setup/install/mac-install/").bold(),
        );
    }
    #[cfg(target_os = "linux")]
    {
        eprintln!("    Install Docker Engine for Linux:");
        eprintln!(
            "      {} {}",
            console::style("▸").cyan(),
            console::style("curl -fsSL https://get.docker.com | sh").bold(),
        );
        eprintln!(
            "      {} or see {}",
            console::style("▸").cyan(),
            console::style("https://docs.docker.com/engine/install/").bold(),
        );
    }
    #[cfg(target_os = "windows")]
    {
        eprintln!("    Install Docker Desktop for Windows:");
        eprintln!(
            "      {} {}",
            console::style("▸").cyan(),
            console::style("winget install Docker.DockerDesktop").bold(),
        );
        eprintln!(
            "      {} or download from {}",
            console::style("▸").cyan(),
            console::style("https://docs.docker.com/desktop/setup/install/windows-install/").bold(),
        );
    }
}

/// Print platform-specific hints for starting the Docker daemon.
fn print_docker_start_hint() {
    #[cfg(target_os = "macos")]
    eprintln!(
        "    {} Open Docker Desktop or run: {}",
        console::style("▸").cyan(),
        console::style("open -a Docker").bold(),
    );
    #[cfg(target_os = "linux")]
    eprintln!(
        "    {} Start the daemon: {}",
        console::style("▸").cyan(),
        console::style("sudo systemctl start docker").bold(),
    );
    #[cfg(target_os = "windows")]
    eprintln!(
        "    {} Open Docker Desktop from the Start menu.",
        console::style("▸").cyan(),
    );
}

/// Read the Playwright version from the `pnpm-workspace.yaml` catalog.
///
/// Parses the semver version from a line like `'@playwright/test': ^1.58.2`,
/// stripping any caret or tilde prefix.
pub fn playwright_version() -> Result<String, String> {
    let content = fs::read_to_string("pnpm-workspace.yaml")
        .map_err(|e| format!("Failed to read pnpm-workspace.yaml: {e}"))?;

    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.contains("@playwright/test") {
            continue;
        }
        // Line format: '@playwright/test': ^1.58.2
        if let Some((_key, value)) = trimmed.rsplit_once(':') {
            let version = value.trim().trim_start_matches('^').trim_start_matches('~');
            let version = version.split_whitespace().next().unwrap_or(version);
            if !version.is_empty() {
                return Ok(version.to_string());
            }
        }
    }

    Err("Could not find @playwright/test version in pnpm-workspace.yaml".into())
}

/// Ensure a Docker image is available locally, pulling it if necessary.
pub fn ensure_docker_image(image: &str) -> Result<(), String> {
    let exists = Command::new("docker")
        .args(["image", "inspect", image])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if exists {
        return Ok(());
    }

    eprintln!(
        "    {} Pulling {} (first time only)…",
        console::style("▸").dim(),
        console::style(image).bold(),
    );
    run_command("docker", &["pull", image], None)
        .map_err(|e| format!("Failed to pull Docker image {image}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::workspace_root;

    #[test]
    fn workspace_root_has_workspace_cargo_toml() {
        let root = match workspace_root() {
            Ok(root) => root,
            Err(message) => panic!("{message}"),
        };

        assert!(
            root.join("Cargo.toml").is_file(),
            "workspace root should contain Cargo.toml at {}",
            root.display()
        );
    }
}
