// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Shared WASM binding errors.

/// Error type used by the WASM binding layer.
#[derive(Debug, thiserror::Error)]
pub(crate) enum WasmError {
    /// Requested entry file was not present in the virtual file map.
    #[cfg(feature = "parser")]
    #[error("Entry file '{0}' not found")]
    MissingEntry(String),

    /// Template parsing or authoring validation failed.
    #[cfg(feature = "parser")]
    #[error("{0}")]
    Parse(#[from] webui_parser::ParserError),

    /// Projection manifest validation or coverage failed.
    #[cfg(feature = "parser")]
    #[error("{0}")]
    Projection(String),

    /// Protocol protobuf bytes could not be decoded or encoded.
    #[error("Protocol error: {0}")]
    Protocol(#[from] webui_protocol::ProtocolError),

    /// State JSON could not be decoded.
    #[cfg(feature = "handler")]
    #[error("State JSON error: {0}")]
    State(serde_json::Error),

    /// Rendering failed.
    #[cfg(feature = "handler")]
    #[error("{0}")]
    Render(#[from] webui_handler::HandlerError),

    /// A requested handler plugin name is not supported.
    #[cfg(feature = "handler")]
    #[error("Unknown plugin: {0}. Use \"webui\", \"fast-v3\", \"fast-v2\", or \"fast\".")]
    UnknownPlugin(String),

    /// The JavaScript render options object was invalid.
    #[cfg(feature = "handler")]
    #[error("Invalid render options: {0}")]
    InvalidOptions(String),
}
