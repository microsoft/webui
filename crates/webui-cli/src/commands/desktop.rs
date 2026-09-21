// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use clap::Args;

use crate::utils::error::CliError;
use crate::utils::output;

const DESKTOP_BINARY_ENV: &str = "WEBUI_DESKTOP_BINARY";
const DEFAULT_DESKTOP_BINARY: &str = "webui-desktop";
const SIDECAR_VERSION_ARG: &str = "--webui-version";
const WEBUI_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Args)]
pub struct DesktopArgs {
    /// Arguments passed through to the desktop sidecar backend
    #[arg(
        value_name = "ARGS",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    pub args: Vec<OsString>,
}

pub fn execute(args: &DesktopArgs) -> Result<()> {
    run(args).inspect_err(|err| {
        output::error(err);
        if let Some(cli_err) = err.chain().find_map(|c| c.downcast_ref::<CliError>()) {
            output::hint(cli_err.hint());
        }
        eprintln!();
    })
}

fn run(args: &DesktopArgs) -> Result<()> {
    let requested = std::env::var_os(DESKTOP_BINARY_ENV);
    let display_binary = requested
        .as_ref()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| DEFAULT_DESKTOP_BINARY.to_string());

    if let Some(binary) = requested.as_ref() {
        if try_sidecar_binary(binary, args)? {
            return Ok(());
        }
        if let Some(path) = workspace_relative_path(binary) {
            if try_sidecar_binary(path.as_os_str(), args)? {
                return Ok(());
            }
        }
    } else if try_sidecar_binary(OsString::from(DEFAULT_DESKTOP_BINARY).as_os_str(), args)? {
        return Ok(());
    }

    if let Some(sibling) = sidecar_next_to_current_exe() {
        if try_sidecar_binary(sibling.as_os_str(), args)? {
            return Ok(());
        }
    }

    if let Some(root) = find_workspace_root() {
        if workspace_has_desktop_sidecar(&root) && try_workspace_sidecar(&root, args)? {
            return Ok(());
        }
    }

    Err(CliError::DesktopBinaryNotFound {
        binary: display_binary,
    }
    .into())
}

fn has_format_arg(args: &[OsString]) -> bool {
    args.iter().any(|arg| {
        arg.to_str()
            .is_some_and(|value| value == "--format" || value.starts_with("--format="))
    })
}

fn append_sidecar_args(command: &mut Command, args: &DesktopArgs) {
    if matches!(output::format(), output::OutputFormat::Json) && !has_format_arg(&args.args) {
        command.arg("--format").arg("json");
    }
    command.args(&args.args);
}

fn try_sidecar_binary(binary: &OsStr, args: &DesktopArgs) -> Result<bool> {
    if !verify_sidecar_version(Command::new(binary), sidecar_path(binary))? {
        return Ok(false);
    }

    let mut command = Command::new(binary);
    append_sidecar_args(&mut command, args);
    run_optional_command(&mut command)
}

fn try_workspace_sidecar(root: &Path, args: &DesktopArgs) -> Result<bool> {
    let sidecar_path = root.join("crates/webui-desktop-runner");
    if !verify_sidecar_version(workspace_sidecar_command(root), sidecar_path)? {
        return Ok(false);
    }

    let mut command = workspace_sidecar_command(root);
    append_sidecar_args(&mut command, args);
    run_optional_command(&mut command)
}

fn workspace_sidecar_command(root: &Path) -> Command {
    let mut command = Command::new("cargo");
    command
        .arg("run")
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .arg("-p")
        .arg("microsoft-webui-desktop-runner")
        .arg("--");
    command
}

fn verify_sidecar_version(mut command: Command, path: PathBuf) -> Result<bool> {
    command.arg(SIDECAR_VERSION_ARG);
    match command.output() {
        Ok(output) if sidecar_version_matches(output.status.success(), &output.stdout) => Ok(true),
        Ok(output) => Err(anyhow::Error::msg(sidecar_version_skew_error(
            path,
            &output.stdout,
        ))),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn sidecar_version_matches(status_success: bool, stdout: &[u8]) -> bool {
    status_success
        && std::str::from_utf8(stdout).is_ok_and(|version| version.trim() == WEBUI_VERSION)
}

fn sidecar_version_skew_error(path: PathBuf, stdout: &[u8]) -> String {
    let reported_version = std::str::from_utf8(stdout)
        .ok()
        .map(str::trim)
        .filter(|version| !version.is_empty())
        .unwrap_or("unavailable");

    format!(
        "Desktop sidecar version mismatch: found {DEFAULT_DESKTOP_BINARY} at {} (reported version: {reported_version}), but webui is version {WEBUI_VERSION}.\nhelp: Set {DESKTOP_BINARY_ENV} to a matching webui-desktop executable, or reinstall WebUI desktop support.",
        path.display()
    )
}

fn sidecar_path(binary: &OsStr) -> PathBuf {
    let binary = Path::new(binary);
    if binary.components().count() > 1 {
        return fs::canonicalize(binary).unwrap_or_else(|_| binary.to_path_buf());
    }

    let Some(paths) = std::env::var_os("PATH") else {
        return binary.to_path_buf();
    };
    for directory in std::env::split_paths(&paths) {
        let candidate = directory.join(binary);
        if candidate.is_file() {
            return fs::canonicalize(&candidate).unwrap_or(candidate);
        }
    }
    binary.to_path_buf()
}

fn run_optional_command(command: &mut Command) -> Result<bool> {
    match command.status() {
        Ok(status) if status.success() => Ok(true),
        Ok(status) => {
            let code = status.code().unwrap_or(1);
            std::process::exit(code);
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn workspace_relative_path(binary: &std::ffi::OsStr) -> Option<PathBuf> {
    let path = Path::new(binary);
    (!path.is_absolute())
        .then(find_workspace_root)
        .flatten()
        .map(|root| root.join(path))
}

fn sidecar_next_to_current_exe() -> Option<PathBuf> {
    let mut path = std::env::current_exe().ok()?;
    path.pop();
    path.push(format!(
        "{}{}",
        DEFAULT_DESKTOP_BINARY,
        std::env::consts::EXE_SUFFIX
    ));
    Some(path)
}

fn find_workspace_root() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let manifest = dir.join("Cargo.toml");
        if fs::read_to_string(&manifest)
            .map(|content| content.contains("[workspace]"))
            .unwrap_or(false)
        {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn workspace_has_desktop_sidecar(root: &Path) -> bool {
    fs::read_to_string(root.join("Cargo.toml"))
        .map(|content| content.contains("crates/*") || content.contains("webui-desktop-runner"))
        .unwrap_or(false)
        && root
            .join("crates/webui-desktop-runner/Cargo.toml")
            .is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_matching_sidecar_version() {
        assert!(sidecar_version_matches(true, WEBUI_VERSION.as_bytes()));
        assert!(sidecar_version_matches(
            true,
            format!("{WEBUI_VERSION}\n").as_bytes()
        ));
    }

    #[test]
    fn rejects_mismatched_sidecar_version_with_actionable_diagnostic() {
        let path = PathBuf::from("/tools/webui-desktop");
        assert!(!sidecar_version_matches(true, b"0.0.28\n"));

        let diagnostic = sidecar_version_skew_error(path, b"0.0.28\n");
        assert_eq!(
            diagnostic,
            format!(
                "Desktop sidecar version mismatch: found webui-desktop at /tools/webui-desktop (reported version: 0.0.28), but webui is version {WEBUI_VERSION}.\nhelp: Set WEBUI_DESKTOP_BINARY to a matching webui-desktop executable, or reinstall WebUI desktop support."
            )
        );
    }

    #[test]
    fn rejects_sidecar_without_version_query_support() {
        assert!(!sidecar_version_matches(false, b""));

        let diagnostic = sidecar_version_skew_error(PathBuf::from("/tools/webui-desktop"), b"");
        assert!(diagnostic.contains("reported version: unavailable"));
        assert!(diagnostic.contains("help:"));
    }
}
