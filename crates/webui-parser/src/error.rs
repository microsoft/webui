// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Error handling for the WebUI parser.

use thiserror::Error;

/// Result type for WebUI parser operations.
pub type Result<T> = std::result::Result<T, ParserError>;

/// Error type for WebUI parser.
#[derive(Debug, Error)]
pub enum ParserError {
    /// Generic error.
    #[error("Generic error: {0}")]
    Generic(String),

    /// I/O error with context.
    #[error("I/O error: {context}: {source}")]
    IO {
        /// What operation failed.
        context: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// Parse error.
    #[error("Parse error: {0}")]
    Parse(String),

    /// Component error.
    #[error("Component error: {0}")]
    Component(String),

    /// CSS error.
    #[error("CSS error: {0}")]
    Css(String),

    /// HTML error.
    #[error("HTML error: {0}")]
    Html(String),

    /// Directive error.
    #[error("Directive error: {0}")]
    Directive(String),

    /// Entity not found error.
    #[error("Not found: {0}")]
    NotFound(String),

    /// TypeScript parse error.
    #[error("TypeScript parse error: {0}")]
    TsParseError(String),
}
