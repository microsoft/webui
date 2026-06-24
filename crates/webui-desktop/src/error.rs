// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::path::PathBuf;

use thiserror::Error;

/// Result type used by the WebUI desktop runtime.
pub type Result<T> = std::result::Result<T, DesktopError>;

/// Errors produced by the runtime-neutral WebUI desktop layer.
#[derive(Debug, Error)]
pub enum DesktopError {
    /// I/O failure while reading or writing desktop runtime data.
    #[error("I/O error while {context}")]
    Io {
        /// Operation that failed.
        context: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// WebUI source build failed.
    #[error("desktop WebUI build failed")]
    Build(#[from] webui::WebUIError),

    /// Handler rendering failed.
    #[error("desktop render failed")]
    Render(#[from] webui_handler::HandlerError),

    /// Protocol serialization or deserialization failed.
    #[error("desktop protocol error")]
    Protocol(#[from] webui_protocol::ProtocolError),

    /// State JSON could not be parsed.
    #[error("failed to parse desktop state JSON from {}", path.display())]
    StateJson {
        /// State file path.
        path: PathBuf,
        /// Underlying JSON parser error.
        #[source]
        source: serde_json::Error,
    },

    /// JSON serialization failed.
    #[error("failed while {context}")]
    Serialization {
        /// Operation that failed.
        context: String,
        /// Underlying JSON error.
        #[source]
        source: serde_json::Error,
    },

    /// Design token loading or resolution failed.
    #[error("failed while {context}")]
    Token {
        /// Operation that failed.
        context: String,
        /// Underlying token error.
        #[source]
        source: webui_tokens::TokenError,
    },

    /// A custom-protocol asset path was unsafe.
    #[error("invalid desktop asset path: {path}")]
    InvalidAssetPath {
        /// Rejected request path.
        path: String,
    },

    /// A Rust route provider pattern is invalid.
    #[error("invalid desktop route pattern: {pattern}")]
    InvalidRoutePattern {
        /// Rejected route pattern.
        pattern: String,
        /// Actionable fix.
        help: String,
    },

    /// A Rust route provider failed.
    #[error("desktop route provider failed for {path}: {message}")]
    RouteProvider {
        /// Request path.
        path: String,
        /// Provider error message.
        message: String,
    },

    /// An asset exceeds the configured in-memory response cap.
    #[error(
        "desktop asset is too large: {} ({} bytes, max {} bytes)",
        path.display(),
        size,
        max_bytes
    )]
    AssetTooLarge {
        /// Asset path.
        path: PathBuf,
        /// Actual size in bytes.
        size: u64,
        /// Configured maximum size.
        max_bytes: u64,
    },

    /// Two bundle inputs resolve to the same output asset path.
    #[error("desktop bundle asset collision: {path}")]
    BundleAssetCollision {
        /// Colliding bundle-relative path.
        path: String,
    },

    /// A bundle or package output path overlaps an input path.
    #[error(
        "desktop output path {} overlaps {input_label} path {}",
        output.display(),
        input.display()
    )]
    OutputPathOverlap {
        /// Output path that would be cleaned or written.
        output: PathBuf,
        /// Input path that must be preserved.
        input: PathBuf,
        /// Human-readable input label.
        input_label: &'static str,
    },

    /// Bundle manifest serialization failed.
    #[error("failed to serialize desktop bundle manifest")]
    ManifestSerialization(#[source] serde_json::Error),

    /// Bundle manifest deserialization failed.
    #[error("failed to parse desktop bundle manifest at {}", path.display())]
    ManifestDeserialization {
        /// Manifest path.
        path: PathBuf,
        /// Underlying parser error.
        #[source]
        source: serde_json::Error,
    },

    /// A requested package target is not implemented by the Rust packager yet.
    #[error("desktop package target '{target}' requires platform packaging tooling")]
    PackageTargetRequiresTooling {
        /// Requested package target.
        target: String,
        /// Tooling that must be available.
        tooling: String,
        /// Actionable help.
        help: String,
    },

    /// Protobuf IPC request payload is larger than the configured cap.
    #[error("desktop IPC payload is too large: {size} bytes (max {max_bytes} bytes)")]
    IpcPayloadTooLarge {
        /// Actual payload size.
        size: usize,
        /// Configured maximum size.
        max_bytes: usize,
    },

    /// Protobuf IPC request decoding failed.
    #[error("failed to decode desktop IPC request")]
    IpcDecode(#[from] prost::DecodeError),

    /// Protobuf IPC response encoding failed.
    #[error("failed to encode desktop IPC response")]
    IpcEncode(#[from] prost::EncodeError),

    /// Platform runtime does not support a required desktop capability.
    #[error("unsupported desktop runtime: {message}")]
    UnsupportedRuntime {
        /// What capability is missing.
        message: String,
        /// Actionable fix.
        help: String,
    },
}

impl DesktopError {
    /// Return an actionable hint for errors that have a clear next step.
    #[must_use]
    pub fn hint(&self) -> Option<&str> {
        match self {
            DesktopError::InvalidAssetPath { .. } => {
                Some("Use a relative asset path without '.', '..', backslashes, NUL bytes, or drive prefixes")
            }
            DesktopError::AssetTooLarge { .. } => {
                Some("Reduce the asset size or raise the desktop max asset size for a trusted app")
            }
            DesktopError::IpcPayloadTooLarge { .. } => {
                Some("Reduce the IPC payload size or raise the IPC limit for a trusted app")
            }
            DesktopError::InvalidRoutePattern { help, .. } => Some(help.as_str()),
            DesktopError::RouteProvider { .. } => {
                Some("Fix the Rust route provider or return a valid route-scoped state object")
            }
            DesktopError::BundleAssetCollision { .. } => {
                Some("Rename the static asset or generated CSS file so each bundle path is unique")
            }
            DesktopError::OutputPathOverlap { .. } => {
                Some("Choose an output directory outside the app, bundle, state, asset, and runner paths")
            }
            DesktopError::PackageTargetRequiresTooling { help, .. } => Some(help.as_str()),
            DesktopError::UnsupportedRuntime { help, .. } => Some(help.as_str()),
            _ => None,
        }
    }

    /// Return a flattened source-chain message for hosts that do not walk
    /// [`std::error::Error::source`].
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
