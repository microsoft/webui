// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Error types for the WebUI library.

use thiserror::Error;

/// Errors that can occur during WebUI build, render, or inspection operations.
#[derive(Debug, Error)]
pub enum WebUIError {
    /// I/O error (file read/write failures).
    #[error("I/O error: {context}")]
    Io {
        /// What operation failed.
        context: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// Component registration failure.
    #[error("Component registration error: {0}")]
    ComponentRegistration(String),

    /// Component discovery failure (npm packages or local paths).
    #[error("Component discovery error: {0}")]
    ComponentDiscovery(String),

    /// HTML/CSS parsing failure with context about which file failed.
    #[error("{context}")]
    Parse {
        /// What was being parsed.
        context: String,
        /// The underlying parse error.
        #[source]
        source: webui_parser::ParserError,
    },

    /// Protocol serialization or deserialization failure.
    #[error("protocol error")]
    Protocol(#[from] webui_protocol::ProtocolError),

    /// JSON serialization failure.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Handler rendering error.
    #[error("rendering error")]
    Rendering(#[from] webui_handler::HandlerError),

    /// Invalid build-option configuration.
    #[error("Invalid build options: {0}")]
    InvalidBuildOptions(String),

    /// Bundler-neutral state projection manifest failure: malformed schema,
    /// stale/missing input or output file, build-ID mismatch, conflicting or
    /// duplicate fragment ownership, missing scripted-component coverage, or
    /// an incompatible plugin. Carries a structured, color-free [`Diagnostic`]
    /// with a stable `PROJ-*` code (see `webui::projection::codes`).
    ///
    /// The diagnostic is boxed so this cold error path does not enlarge
    /// `Result<_, WebUIError>` on the common success path.
    #[error("{0}")]
    Projection(Box<webui_parser::Diagnostic>),
}

impl WebUIError {
    /// The full, single-line message including the source chain, e.g.
    /// `Failed to parse index.html: Directive error: Invalid for each: x`.
    ///
    /// Each error layer's [`std::fmt::Display`] describes only its own level
    /// (the `#[source]` chain carries the rest), so the CLI and dev server can
    /// use anyhow's `{:#}` formatting without repetition. Hosts that surface a
    /// flat string instead of walking the chain — Node, FFI — use this helper
    /// to get the same complete message.
    #[must_use]
    pub fn chain_message(&self) -> String {
        let mut message = self.to_string();
        let mut source = std::error::Error::source(self);
        while let Some(err) = source {
            message.push_str(": ");
            message.push_str(&err.to_string());
            source = err.source();
        }
        message
    }
}
