// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Error handling for the WebUI parser.

use crate::diagnostic::Diagnostic;
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
    #[error("I/O error: {context}")]
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

    /// A component supplied its own `<template>` wrapper under
    /// `CssStrategy::Module` but did not declare a
    /// `shadowrootadoptedstylesheets` attribute. The framework cannot
    /// silently inject the attribute without overriding developer intent
    /// on the wrapper, so authoring help is surfaced at build time.
    #[error(
        "Component <{tag}> supplies its own <template> wrapper under --css=module but is \
         missing the `shadowrootadoptedstylesheets` attribute, which is required to wire \
         the component's CSS module into its shadow root. Add \
         `shadowrootadoptedstylesheets=\"{tag}\"` to the <template> tag, or remove the \
         <template> wrapper to let the framework manage it automatically."
    )]
    MissingAdoptedStylesheets {
        /// The component tag name (e.g. `mp-app`).
        tag: String,
    },

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

    /// A build-time template-authoring mistake (e.g. an invalid `@event`
    /// handler or a non-braced `w-ref`). Carries a structured [`Diagnostic`]
    /// whose [`std::fmt::Display`] is an actionable, color-free report. The CLI
    /// colorizes it; FFI/Node/WASM forward the plain text to their host error
    /// channel so the application can handle it.
    ///
    /// The diagnostic is boxed so this cold error path does not enlarge
    /// `Result<_, ParserError>` on the common success path.
    #[error("{0}")]
    Template(Box<Diagnostic>),
}

impl From<Diagnostic> for ParserError {
    fn from(diagnostic: Diagnostic) -> Self {
        ParserError::Template(Box::new(diagnostic))
    }
}
