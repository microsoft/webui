// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Result};
use clap::Args;

use crate::utils::error::CliError;
use crate::utils::output;

mod npm;

const WEBUI_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Sidecar {
    Desktop,
    Press,
}

impl Sidecar {
    fn binary(self) -> &'static str {
        match self {
            Self::Desktop => "webui-desktop",
            Self::Press => "webui-press",
        }
    }

    fn override_env(self) -> &'static str {
        match self {
            Self::Desktop => "WEBUI_DESKTOP_BINARY",
            Self::Press => "WEBUI_PRESS_BINARY_PATH",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Desktop => "Desktop",
            Self::Press => "Press",
        }
    }
}

#[derive(Args)]
pub struct SidecarArgs {
    /// Arguments passed through to the native sidecar
    #[arg(
        value_name = "ARGS",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    pub args: Vec<OsString>,
}

pub fn execute(sidecar: Sidecar, args: &SidecarArgs) -> Result<()> {
    run(sidecar, args).inspect_err(|err| {
        output::error(err);
        if let Some(cli_err) = err.chain().find_map(|c| c.downcast_ref::<CliError>()) {
            output::hint(cli_err.hint());
        }
        eprintln!();
    })
}

fn run(sidecar: Sidecar, args: &SidecarArgs) -> Result<()> {
    if sidecar == Sidecar::Press && matches!(output::format(), output::OutputFormat::Json) {
        bail!("webui press does not support --format json.\nhelp: Use --format human.");
    }
    let requested = std::env::var_os(sidecar.override_env());
    let display_binary = requested
        .as_ref()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| sidecar.binary().to_string());

    if let Some(binary) = requested.as_ref() {
        if try_sidecar_binary(sidecar, binary, args)? {
            return Ok(());
        }
        if let Some(path) = workspace_relative_path(binary) {
            if try_sidecar_binary(sidecar, path.as_os_str(), args)? {
                return Ok(());
            }
        }
    } else {
        if try_installed_sidecar(sidecar, args)? {
            return Ok(());
        }
        if try_sidecar_binary(sidecar, OsStr::new(sidecar.binary()), args)? {
            return Ok(());
        }
    }

    if let Some(sibling) = sidecar_next_to_current_exe(sidecar) {
        if try_sidecar_binary(sidecar, sibling.as_os_str(), args)? {
            return Ok(());
        }
    }

    if let Some(root) = find_workspace_root() {
        if root
            .join("crates")
            .join(sidecar.binary())
            .join("Cargo.toml")
            .is_file()
            && try_workspace_sidecar(sidecar, &root, args)?
        {
            return Ok(());
        }
    }

    Err(match sidecar {
        Sidecar::Desktop => CliError::DesktopBinaryNotFound {
            binary: display_binary,
        },
        Sidecar::Press => CliError::PressBinaryNotFound {
            binary: display_binary,
        },
    }
    .into())
}

fn try_installed_sidecar(sidecar: Sidecar, args: &SidecarArgs) -> Result<bool> {
    for start in [std::env::current_dir().ok(), std::env::current_exe().ok()]
        .into_iter()
        .flatten()
    {
        if let Some(binary) = npm::sidecar_near(sidecar, &start)? {
            return try_sidecar_binary(sidecar, binary.as_os_str(), args);
        }
    }
    Ok(false)
}

fn has_format_arg(args: &[OsString]) -> bool {
    args.iter().any(|arg| {
        arg.to_str()
            .is_some_and(|value| value == "--format" || value.starts_with("--format="))
    })
}

fn append_sidecar_args(command: &mut Command, args: &SidecarArgs) {
    if matches!(output::format(), output::OutputFormat::Json) && !has_format_arg(&args.args) {
        command.arg("--format").arg("json");
    }
    command.args(&args.args);
}

fn try_sidecar_binary(sidecar: Sidecar, binary: &OsStr, args: &SidecarArgs) -> Result<bool> {
    if !verify_sidecar_version(sidecar, Command::new(binary), sidecar_path(binary))? {
        return Ok(false);
    }

    let mut command = Command::new(binary);
    append_sidecar_args(&mut command, args);
    run_optional_command(&mut command)
}

fn try_workspace_sidecar(sidecar: Sidecar, root: &Path, args: &SidecarArgs) -> Result<bool> {
    let sidecar_path = root.join("crates").join(sidecar.binary());
    if !verify_sidecar_version(
        sidecar,
        workspace_sidecar_command(sidecar, root),
        sidecar_path,
    )? {
        return Ok(false);
    }

    let mut command = workspace_sidecar_command(sidecar, root);
    append_sidecar_args(&mut command, args);
    run_optional_command(&mut command)
}

fn workspace_sidecar_command(sidecar: Sidecar, root: &Path) -> Command {
    let mut command = Command::new("cargo");
    command
        .arg("run")
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .arg("-p")
        .arg(format!("microsoft-{}", sidecar.binary()));
    if sidecar == Sidecar::Desktop {
        command.args(["--features", "cli"]);
    }
    command.args(["--bin", sidecar.binary(), "--"]);
    command
}

fn verify_sidecar_version(sidecar: Sidecar, mut command: Command, path: PathBuf) -> Result<bool> {
    command.arg(if sidecar == Sidecar::Desktop {
        "--webui-version"
    } else {
        "--version"
    });
    match command.output() {
        Ok(output) if sidecar_version_matches(sidecar, output.status.success(), &output.stdout) => {
            Ok(true)
        }
        Ok(output) => Err(anyhow::Error::msg(sidecar_version_skew_error(
            sidecar,
            path,
            &output.stdout,
        ))),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn sidecar_version_matches(sidecar: Sidecar, status_success: bool, stdout: &[u8]) -> bool {
    status_success
        && std::str::from_utf8(stdout).is_ok_and(|version| {
            let version = version.trim();
            match sidecar {
                Sidecar::Desktop => version == WEBUI_VERSION,
                Sidecar::Press => version.strip_prefix("webui-press ") == Some(WEBUI_VERSION),
            }
        })
}

fn sidecar_version_skew_error(sidecar: Sidecar, path: PathBuf, stdout: &[u8]) -> String {
    let reported_version = std::str::from_utf8(stdout)
        .ok()
        .map(str::trim)
        .filter(|version| !version.is_empty())
        .unwrap_or("unavailable");

    format!(
        "{} sidecar version mismatch: found {} at {} (reported version: {reported_version}), but webui is version {WEBUI_VERSION}.\nhelp: Set {} to a matching executable, or reinstall @microsoft/{}.",
        sidecar.label(), sidecar.binary(), path.display(), sidecar.override_env(), sidecar.binary()
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

fn sidecar_next_to_current_exe(sidecar: Sidecar) -> Option<PathBuf> {
    let mut path = std::env::current_exe().ok()?;
    path.pop();
    path.push(format!(
        "{}{}",
        sidecar.binary(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_sidecar_explicitly_enables_sdk_cli_feature() {
        let root = Path::new("workspace");
        let command = workspace_sidecar_command(Sidecar::Desktop, root);
        let args: Vec<_> = command.get_args().map(OsStr::to_os_string).collect();
        assert_eq!(command.get_program(), OsStr::new("cargo"));
        assert_eq!(
            args,
            [
                OsString::from("run"),
                OsString::from("--manifest-path"),
                root.join("Cargo.toml").into_os_string(),
                OsString::from("-p"),
                OsString::from("microsoft-webui-desktop"),
                OsString::from("--features"),
                OsString::from("cli"),
                OsString::from("--bin"),
                OsString::from("webui-desktop"),
                OsString::from("--"),
            ]
        );
        let press = workspace_sidecar_command(Sidecar::Press, root);
        assert!(!press.get_args().any(|arg| arg == "--features"));
        assert!(press.get_args().any(|arg| arg == "microsoft-webui-press"));
    }

    #[test]
    fn accepts_matching_sidecar_version() {
        assert!(sidecar_version_matches(
            Sidecar::Desktop,
            true,
            WEBUI_VERSION.as_bytes()
        ));
        assert!(sidecar_version_matches(
            Sidecar::Desktop,
            true,
            format!("{WEBUI_VERSION}\n").as_bytes()
        ));
        assert!(sidecar_version_matches(
            Sidecar::Press,
            true,
            format!("webui-press {WEBUI_VERSION}\n").as_bytes()
        ));
        assert!(!sidecar_version_matches(
            Sidecar::Press,
            true,
            b"webui-press 0.0.0\n"
        ));
    }

    #[test]
    fn rejects_mismatched_sidecar_version_with_actionable_diagnostic() {
        let path = PathBuf::from("/tools/webui-desktop");
        assert!(!sidecar_version_matches(
            Sidecar::Desktop,
            true,
            b"0.0.28\n"
        ));

        let diagnostic = sidecar_version_skew_error(Sidecar::Desktop, path, b"0.0.28\n");
        assert_eq!(
            diagnostic,
            format!(
                "Desktop sidecar version mismatch: found webui-desktop at /tools/webui-desktop (reported version: 0.0.28), but webui is version {WEBUI_VERSION}.\nhelp: Set WEBUI_DESKTOP_BINARY to a matching executable, or reinstall @microsoft/webui-desktop."
            )
        );
    }

    #[test]
    fn rejects_sidecar_without_version_query_support() {
        assert!(!sidecar_version_matches(Sidecar::Desktop, false, b""));

        let diagnostic = sidecar_version_skew_error(
            Sidecar::Desktop,
            PathBuf::from("/tools/webui-desktop"),
            b"",
        );
        assert!(diagnostic.contains("reported version: unavailable"));
        assert!(diagnostic.contains("help:"));
    }
}
