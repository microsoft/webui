//! Error types for the WebUI library.

use thiserror::Error;

/// Errors that can occur during WebUI build, render, or inspection operations.
#[derive(Debug, Error)]
pub enum WebUIError {
    /// I/O error (file read/write failures).
    #[error("I/O error: {0}")]
    Io(String),

    /// Invalid or unknown parser plugin.
    #[error("Unknown plugin: {0}")]
    InvalidPlugin(String),

    /// Component registration failure.
    #[error("Component registration error: {0}")]
    ComponentRegistration(String),

    /// Component discovery failure (npm packages or local paths).
    #[error("Component discovery error: {0}")]
    ComponentDiscovery(String),

    /// HTML/CSS parsing failure.
    #[error("Parse error: {0}")]
    Parse(String),

    /// Protocol serialization or deserialization failure.
    #[error("Protocol error: {0}")]
    Protocol(String),

    /// JSON serialization failure.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Handler rendering error.
    #[error("Rendering error: {0}")]
    Rendering(String),
}
