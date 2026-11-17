// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Resolve only an explicitly selected or adjacent standalone native toolchain.

#[path = "toolchain/contract.rs"]
mod contract;
#[path = "toolchain/probe.rs"]
mod probe;
#[path = "toolchain/validation.rs"]
mod validation;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Validated, release-matched native TypeScript tooling.
#[derive(Debug)]
pub(crate) struct Toolchain {
    /// Absolute path to the native executable; never a JavaScript launcher.
    pub(crate) compiler: PathBuf,
}

/// Actionable standalone toolchain failures, independent of presentation.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ToolchainError {
    /// Unsupported native release target.
    #[error("the standalone WebUI toolchain does not support target {0}")]
    Unsupported(String),
    /// Invalid release identity, inventory, native format, or integrity.
    #[error("invalid standalone WebUI toolchain at {}: {reason}", path.display())]
    Invalid { path: PathBuf, reason: String },
    /// A required local artifact could not be read.
    #[error("could not read standalone WebUI toolchain file {}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The integrity-validated executable failed its native version probe.
    #[error("native TypeScript compiler check failed at {}: {reason}", path.display())]
    Compiler { path: PathBuf, reason: String },
}

impl ToolchainError {
    /// Stable machine-readable diagnostic identifier.
    #[must_use]
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Unsupported(_) => "toolchain-unsupported-target",
            Self::Invalid { .. } => "toolchain-invalid",
            Self::Read { .. } => "toolchain-read-failed",
            Self::Compiler { .. } => "toolchain-compiler-failed",
        }
    }

    /// Recovery guidance; an explicit override must never silently fall back.
    #[must_use]
    pub(crate) fn help(&self) -> &'static str {
        concat!(
            "Extract the complete WebUI ", env!("CARGO_PKG_VERSION"),
            " standalone toolchain for this target and verify its SHA-256 sidecar. ",
            "Keep bin/webui, compiler, notices and toolchain.json together, or set ",
            "WEBUI_TOOLCHAIN_DIR to that archive root. The override is authoritative. ",
            "TypeScript 7.0.2 must be native; Node, npm, PATH tsc and automatic downloads are not used."
        )
    }

    /// BSD-style process status for the CLI's common error renderer.
    #[must_use]
    pub(crate) fn exit_code(&self) -> i32 {
        match self {
            Self::Unsupported(_) => 69,
            Self::Invalid { .. } | Self::Compiler { .. } => 65,
            Self::Read { .. } => 74,
        }
    }
}

/// Resolve and validate native tooling without package-manager discovery.
pub(crate) fn resolve() -> Result<Toolchain, ToolchainError> {
    resolve_with(std::env::var_os("WEBUI_TOOLCHAIN_DIR").as_deref(), || {
        std::env::current_exe().map_err(|source| ToolchainError::Read {
            path: PathBuf::from("current executable"),
            source,
        })
    })
}

fn resolve_with(
    explicit: Option<&OsStr>,
    executable: impl FnOnce() -> Result<PathBuf, ToolchainError>,
) -> Result<Toolchain, ToolchainError> {
    let root = match explicit {
        Some(value) if value.is_empty() => {
            return Err(invalid(
                Path::new(""),
                "WEBUI_TOOLCHAIN_DIR must not be empty",
            ));
        }
        Some(value) => PathBuf::from(value),
        None => {
            let executable = executable()?;
            executable
                .parent()
                .and_then(Path::parent)
                .ok_or_else(|| invalid(&executable, "expected archive-root/bin/webui"))?
                .to_owned()
        }
    };
    validation::validate(&root)
}

#[cold]
#[inline(never)]
fn invalid(path: &Path, reason: impl Into<String>) -> ToolchainError {
    ToolchainError::Invalid {
        path: path.to_owned(),
        reason: reason.into(),
    }
}

#[cfg(test)]
#[path = "toolchain/tests.rs"]
mod tests;
