// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fmt;
use thiserror::Error;

/// Errors that can occur during token loading, resolution, or CSS generation.
#[derive(Debug, Error)]
pub enum TokenError {
    /// Token file could not be read from disk.
    #[error("Failed to read token file {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },

    /// Token file contained invalid JSON.
    #[error("Invalid token JSON in {path}: {source}")]
    InvalidJson {
        path: String,
        source: serde_json::Error,
    },

    /// Token file schema is invalid (e.g., missing `themes` key, wrong types).
    #[error("Invalid token file schema: {0}")]
    Schema(String),

    /// A cyclic dependency was detected among token values.
    #[error("Cyclic token dependency detected: {0}")]
    CyclicDependency(String),
}

impl TokenError {
    /// Create an IO error with the file path context.
    pub(crate) fn io(path: &std::path::Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.display().to_string(),
            source,
        }
    }

    /// Create a JSON parse error with the file path context.
    pub(crate) fn json(path: &std::path::Path, source: serde_json::Error) -> Self {
        Self::InvalidJson {
            path: path.display().to_string(),
            source,
        }
    }

    /// Format a cycle path for display.
    pub(crate) fn cycle(chain: &[&str]) -> Self {
        Self::CyclicDependency(
            chain
                .iter()
                .map(|s| format!("--{s}"))
                .collect::<Vec<_>>()
                .join(" → "),
        )
    }
}

/// Result type alias for token operations.
pub type Result<T> = std::result::Result<T, TokenError>;

/// Warnings produced during token resolution (non-fatal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenWarning {
    /// A token required by the protocol was not found in the token file.
    MissingToken { theme: String, token: String },

    /// A token value references another token via `var(--x)` that is not
    /// defined in the token file.
    MissingDependency {
        theme: String,
        token: String,
        dependency: String,
    },
}

impl fmt::Display for TokenWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingToken { theme, token } => {
                write!(
                    f,
                    "Token --{token} required by protocol but not found in theme '{theme}'"
                )
            }
            Self::MissingDependency {
                theme,
                token,
                dependency,
            } => {
                write!(
                    f,
                    "Token --{token} in theme '{theme}' references --{dependency} which is not defined"
                )
            }
        }
    }
}
