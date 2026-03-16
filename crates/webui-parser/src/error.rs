// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Error handling for the WebUI parser.

use std::fmt;

/// Result type for WebUI parser operations.
pub type Result<T> = std::result::Result<T, ParserError>;

/// Error type for WebUI parser.
#[derive(Debug)]
pub enum ParserError {
    /// Generic error.
    Generic(String),

    /// I/O error.
    IO(String),

    /// Parse error.
    Parse(String),

    /// Component error.
    Component(String),

    /// CSS error.
    Css(String),

    /// HTML error.
    Html(String),

    /// Directive error.
    Directive(String),

    /// Entity not found error.
    NotFound(String),

    /// TypeScript parse error.
    TsParseError(String),
}

impl fmt::Display for ParserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Generic(msg) => write!(f, "Generic error: {}", msg),
            Self::IO(msg) => write!(f, "I/O error: {}", msg),
            Self::Parse(msg) => write!(f, "Parse error: {}", msg),
            Self::Component(msg) => write!(f, "Component error: {}", msg),
            Self::Css(msg) => write!(f, "CSS error: {}", msg),
            Self::Html(msg) => write!(f, "HTML error: {}", msg),
            Self::Directive(msg) => write!(f, "Directive error: {}", msg),
            Self::NotFound(msg) => write!(f, "Not found: {}", msg),
            Self::TsParseError(msg) => write!(f, "TypeScript parse error: {}", msg),
        }
    }
}

impl std::error::Error for ParserError {}
