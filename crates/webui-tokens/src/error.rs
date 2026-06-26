// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

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

    /// A flat token required by a caller is missing from a theme.
    #[error(
        "Token --{token} required by caller but not found in theme '{theme}'. Add --{token} to the theme or remove it from the required list."
    )]
    MissingToken {
        /// Theme that is missing the token.
        theme: String,
        /// Token name without the `--` prefix.
        token: String,
    },
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
}

/// Result type alias for token operations.
pub type Result<T> = std::result::Result<T, TokenError>;
