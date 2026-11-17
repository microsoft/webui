// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod environment;
mod validation;

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::utils::output::{self, OutputFormat};

#[derive(Args)]
pub(crate) struct SdkArgs {
    #[command(subcommand)]
    command: SdkCommand,
}

#[derive(Subcommand)]
enum SdkCommand {
    /// Check the version, platform and integrity of the standalone SDK
    Check {
        /// Application directory containing .webui/sdk
        #[arg(long, default_value = ".")]
        project: PathBuf,
    },
    /// Run a native command with SDK include, link and loader paths
    Exec {
        /// Application directory, also used as the command's working directory
        #[arg(long, default_value = ".")]
        project: PathBuf,
        /// Executable and arguments, after --; no shell is used
        #[arg(last = true, required = true, num_args = 1..)]
        command: Vec<OsString>,
    },
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SdkError {
    #[error("the standalone WebUI SDK does not support {0}")]
    Unsupported(String),
    #[error("invalid standalone WebUI SDK at {}: {reason}", path.display())]
    Invalid { path: PathBuf, reason: String },
    #[error("could not read standalone WebUI SDK file {}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not start SDK command {program:?}")]
    Launch {
        program: OsString,
        #[source]
        source: std::io::Error,
    },
    #[cfg(not(unix))]
    #[error("SDK command terminated without an exit code")]
    Interrupted,
}

impl SdkError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Unsupported(_) => "sdk-unsupported-platform",
            Self::Invalid { .. } => "sdk-invalid",
            Self::Read { .. } => "sdk-read-failed",
            Self::Launch { .. } => "sdk-command-failed",
            #[cfg(not(unix))]
            Self::Interrupted => "sdk-command-interrupted",
        }
    }

    pub(crate) fn help(&self) -> &'static str {
        match self {
            Self::Unsupported(_) => "Use a supported x64/ARM64 macOS, GNU Linux, or MSVC Windows toolchain and its matching standalone SDK.",
            Self::Invalid { .. } | Self::Read { .. } => concat!(
                "Download the WebUI ", env!("CARGO_PKG_VERSION"),
                " standalone SDK and its SHA-256 sidecar from the matching GitHub release. ",
                "Verify the archive and extract its root into .webui/sdk, or set WEBUI_SDK_DIR to the extracted directory. ",
                "An explicit override is authoritative; SDKs are never downloaded or taken from npm."
            ),
            Self::Launch { .. } => "Check that the requested native executable is installed and the project working directory exists.",
            #[cfg(not(unix))]
            Self::Interrupted => "Inspect the native command's diagnostics and retry after resolving its termination.",
        }
    }

    pub(crate) fn exit_code(&self) -> i32 {
        match self {
            Self::Unsupported(_) => 69,
            Self::Invalid { .. } => 65,
            Self::Read { .. } => 74,
            Self::Launch { .. } => 1,
            #[cfg(not(unix))]
            Self::Interrupted => 1,
        }
    }
}

#[cold]
#[inline(never)]
fn invalid(path: &std::path::Path, reason: impl Into<String>) -> SdkError {
    SdkError::Invalid {
        path: path.to_owned(),
        reason: reason.into(),
    }
}

pub(crate) fn execute(args: &SdkArgs) -> Result<i32> {
    run(args).inspect_err(output::error)
}

pub(crate) fn configure(command: &mut Command, project: &std::path::Path) -> Result<()> {
    let sdk = validation::resolve(project, std::env::var_os("WEBUI_SDK_DIR").as_deref())?;
    environment::configure(command, &sdk.directory)?;
    Ok(())
}

fn run(args: &SdkArgs) -> Result<i32> {
    let project = match &args.command {
        SdkCommand::Check { project } | SdkCommand::Exec { project, .. } => project,
    };
    let sdk = validation::resolve(project, std::env::var_os("WEBUI_SDK_DIR").as_deref())?;
    match &args.command {
        SdkCommand::Check { .. } => {
            if matches!(output::format(), OutputFormat::Json) {
                println!("{}", serde_json::to_string(&sdk)?);
            } else {
                output::header("WebUI SDK");
                output::field("Directory", &sdk.directory.display());
                output::field("Version", &sdk.version);
                output::field("Target", &sdk.target);
                output::success("Standalone SDK validated");
            }
            Ok(0)
        }
        SdkCommand::Exec { command, .. } => {
            let Some((program, arguments)) = command.split_first() else {
                anyhow::bail!("SDK exec requires an executable after --");
            };
            let mut child = Command::new(program);
            child.args(arguments).current_dir(project);
            environment::configure(&mut child, &sdk.directory)?;
            launch(child, program)
        }
    }
}

#[cfg(unix)]
fn launch(mut command: Command, program: &std::ffi::OsStr) -> Result<i32> {
    use std::os::unix::process::CommandExt;
    Err(SdkError::Launch {
        program: program.to_owned(),
        source: command.exec(),
    }
    .into())
}

#[cfg(not(unix))]
fn launch(mut command: Command, program: &std::ffi::OsStr) -> Result<i32> {
    let status = command.status().map_err(|source| SdkError::Launch {
        program: program.to_owned(),
        source,
    })?;
    status.code().ok_or_else(|| SdkError::Interrupted.into())
}
