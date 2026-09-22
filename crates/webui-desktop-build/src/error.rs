// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::path::PathBuf;

/// Recoverable, plain-text build diagnostics with stable machine-readable codes.
#[derive(Debug, thiserror::Error)]
pub enum GenerateError {
    /// Invalid schema, including its descriptor-qualified source context.
    #[error("{code}: {context}: {message}\nhelp: {help}")]
    Schema {
        /// Stable diagnostic identifier.
        code: &'static str,
        /// Source file and fully qualified descriptor name.
        context: String,
        /// What failed.
        message: String,
        /// How to repair the schema.
        help: String,
    },
    /// An external compiler is missing or failed.
    #[error("ipc-tool: {tool}: {message}\nhelp: {help}")]
    Tool {
        /// Tool name.
        tool: String,
        /// Bounded compiler diagnostic.
        message: String,
        /// Installation or configuration guidance.
        help: String,
    },
    /// Filesystem failure with the owning path.
    #[error("ipc-io: {path}: {source}\nhelp: check the path and filesystem permissions")]
    Io {
        /// Path being accessed.
        path: PathBuf,
        /// Original I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// Check mode found stale or absent output.
    #[error("ipc-drift: generated artifacts differ: {paths:?}\nhelp: regenerate and commit all outputs together")]
    Drift {
        /// Stale or missing artifacts.
        paths: Vec<PathBuf>,
    },
}

impl GenerateError {
    /// Stable code suitable for CLI JSON diagnostics.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Schema { code, .. } => code,
            Self::Tool { .. } => "ipc-tool",
            Self::Io { .. } => "ipc-io",
            Self::Drift { .. } => "ipc-drift",
        }
    }
}

#[cold]
pub(crate) fn schema(
    code: &'static str,
    context: impl Into<String>,
    message: impl Into<String>,
    help: impl Into<String>,
) -> GenerateError {
    GenerateError::Schema {
        code,
        context: context.into(),
        message: message.into(),
        help: help.into(),
    }
}

pub(crate) fn io(path: &std::path::Path, source: std::io::Error) -> GenerateError {
    GenerateError::Io {
        path: path.into(),
        source,
    }
}
