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

    /// HTML/CSS parsing failure.
    #[error("Parse error: {0}")]
    Parse(#[from] webui_parser::ParserError),

    /// Protocol serialization or deserialization failure.
    #[error("Protocol error: {0}")]
    Protocol(#[from] webui_protocol::ProtocolError),

    /// JSON serialization failure.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Handler rendering error.
    #[error("Rendering error: {0}")]
    Rendering(#[from] webui_handler::HandlerError),
}
