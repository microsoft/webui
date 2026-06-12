// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Typed CLI-level errors.
//!
//! These represent the handful of failures the CLI itself raises before (or
//! around) a build/serve — missing inputs, an occupied port, an unreadable
//! entry file. Modelling them as a type (rather than bare `anyhow!` strings)
//! lets the command layer attach an actionable `hint:` and pick a meaningful
//! process exit code by matching the variant, instead of fragile substring
//! checks against the message text.

use std::fmt;

/// A CLI-level failure with an actionable hint and a stable exit code.
#[derive(Debug)]
pub enum CliError {
    /// The positional app folder argument does not exist.
    AppFolderNotFound {
        /// The path that was not found.
        path: String,
    },

    /// The `--state` JSON file does not exist.
    StateFileNotFound {
        /// The path that was not found.
        path: String,
    },

    /// The `--servedir` static-assets directory does not exist.
    ServeDirNotFound {
        /// The path that was not found.
        path: String,
    },

    /// The requested port is already bound by another process.
    PortInUse {
        /// The conflicting port.
        port: u16,
    },

    /// The entry file could not be read.
    EntryReadFailed {
        /// The path that could not be read.
        path: String,
    },
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::AppFolderNotFound { path } => write!(f, "App folder not found: {path}"),
            CliError::StateFileNotFound { path } => write!(f, "State file not found: {path}"),
            CliError::ServeDirNotFound { path } => write!(f, "Serve directory not found: {path}"),
            CliError::PortInUse { port } => {
                write!(f, "Port {port} on 127.0.0.1 is already in use")
            }
            CliError::EntryReadFailed { path } => write!(f, "Failed to read entry file: {path}"),
        }
    }
}

impl std::error::Error for CliError {}

impl CliError {
    /// A short, actionable next step shown to the developer as `hint:`.
    #[must_use]
    pub fn hint(&self) -> &'static str {
        match self {
            CliError::AppFolderNotFound { .. } => "Check that the app folder path exists",
            CliError::StateFileNotFound { .. } => "Pass a valid --state path to a JSON file",
            CliError::ServeDirNotFound { .. } => "Pass a valid --servedir path for static assets",
            CliError::PortInUse { .. } => {
                "Stop the process using that port, or rerun with --port <free-port>. Previous dev \
                 sessions may have left a server running."
            }
            CliError::EntryReadFailed { .. } => {
                "Use --entry <file> to specify a different entry file"
            }
        }
    }

    /// The process exit code for this failure, following the BSD `sysexits.h`
    /// conventions so scripts and CI can branch on the cause.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        match self {
            // Missing input file/dir → EX_NOINPUT.
            CliError::AppFolderNotFound { .. }
            | CliError::StateFileNotFound { .. }
            | CliError::ServeDirNotFound { .. }
            | CliError::EntryReadFailed { .. } => 66,
            // A required service (the port) is unavailable → EX_UNAVAILABLE.
            CliError::PortInUse { .. } => 69,
        }
    }
}

/// Map a top-level CLI error to a process exit code, following BSD
/// `sysexits.h` so scripts, CI, and tooling can branch on the cause:
///
/// - `66` (`EX_NOINPUT`) / `69` (`EX_UNAVAILABLE`) — CLI input/port failures
///   (see [`CliError::exit_code`]).
/// - `65` (`EX_DATAERR`) — a template authoring or parse error (the source is
///   malformed).
/// - `74` (`EX_IOERR`) — an I/O failure reading or writing files.
/// - `1` — any other failure.
///
/// (clap already exits with `2` for argument/usage errors before this runs.)
#[must_use]
pub fn exit_code(err: &anyhow::Error) -> i32 {
    if let Some(cli) = err.chain().find_map(|c| c.downcast_ref::<CliError>()) {
        return i32::from(cli.exit_code());
    }
    if let Some(web) = err
        .chain()
        .find_map(|c| c.downcast_ref::<webui::WebUIError>())
    {
        return match web {
            webui::WebUIError::Parse { .. }
            | webui::WebUIError::Serialization(_)
            | webui::WebUIError::InvalidBuildOptions(_) => 65,
            webui::WebUIError::Io { .. } => 74,
            _ => 1,
        };
    }
    if err.chain().any(|c| c.is::<std::io::Error>()) {
        return 74;
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_error_hints_and_codes() {
        let missing = CliError::AppFolderNotFound { path: "x".into() };
        assert_eq!(missing.exit_code(), 66);
        assert!(missing.hint().contains("app folder"));

        let port = CliError::PortInUse { port: 3000 };
        assert_eq!(port.exit_code(), 69);
        assert!(port.hint().contains("--port"));
    }

    #[test]
    fn exit_code_classifies_cli_error_through_chain() {
        let err = anyhow::Error::new(CliError::StateFileNotFound {
            path: "s.json".into(),
        })
        .context("while starting the dev server");
        assert_eq!(exit_code(&err), 66);
    }

    #[test]
    fn exit_code_for_parse_error_is_dataerr() {
        let err: anyhow::Error = webui::WebUIError::InvalidBuildOptions("bad".into()).into();
        assert_eq!(exit_code(&err), 65);
    }

    #[test]
    fn exit_code_for_plain_io_error_is_ioerr() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "nope");
        let err = anyhow::Error::new(io).context("reading file");
        assert_eq!(exit_code(&err), 74);
    }

    #[test]
    fn exit_code_default_is_one() {
        let err = anyhow::anyhow!("something generic");
        assert_eq!(exit_code(&err), 1);
    }
}
