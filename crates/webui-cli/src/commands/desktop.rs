// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::ffi::OsString;
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

fn try_sidecar_binary(binary: &std::ffi::OsStr, args: &DesktopArgs) -> Result<bool> {
    let mut command = Command::new(binary);
    append_sidecar_args(&mut command, args);
    run_optional_command(&mut command)
}

fn try_workspace_sidecar(root: &Path, args: &DesktopArgs) -> Result<bool> {
    let mut command = Command::new("cargo");
    command
        .arg("run")
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .arg("-p")
        .arg("microsoft-webui-desktop-cli")
        .arg("--");
    append_sidecar_args(&mut command, args);
    run_optional_command(&mut command)
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
        .map(|content| content.contains("crates/*") || content.contains("webui-desktop-cli"))
        .unwrap_or(false)
        && root.join("crates/webui-desktop-cli/Cargo.toml").is_file()
}
