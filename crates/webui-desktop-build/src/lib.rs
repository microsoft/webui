// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Build-time typed IPC bindings for WebUI desktop applications.

mod artifacts;
mod compiler;
mod emit;
mod error;
mod evolution;
mod model;
mod names;

use std::path::PathBuf;

pub use error::GenerateError;

/// Inputs and destinations for deterministic application IPC generation.
#[derive(Clone, Debug)]
pub struct GenerateConfig {
    /// Application proto files (not the SDK options schema).
    pub roots: Vec<PathBuf>,
    /// Protobuf import search directories.
    pub includes: Vec<PathBuf>,
    /// Destination for `ipc.rs`, `ipc_messages.rs`, and prost modules.
    pub rust_out: PathBuf,
    /// Destination for `ipc.ts` and ts-proto modules.
    pub ts_out: PathBuf,
    /// Checked-in compatibility history, conventionally `ipc-schema.lock.json`.
    pub lock_file: PathBuf,
    /// Compare all artifacts without modifying existing files.
    pub check: bool,
    /// Explicit protoc executable, or `protoc` on PATH.
    pub protoc: Option<PathBuf>,
    /// Explicit ts-proto 2.12.3 plugin, or `protoc-gen-ts_proto` on PATH.
    pub ts_proto_plugin: Option<PathBuf>,
}

/// Successfully generated (or verified) artifact destinations.
#[derive(Clone, Debug)]
pub struct GeneratedFiles {
    /// All Rust output files.
    pub rust: Vec<PathBuf>,
    /// All TypeScript output files.
    pub typescript: Vec<PathBuf>,
    /// Normalized semantic manifest.
    pub manifest: PathBuf,
    /// Inventory used to detect and remove obsolete generated modules.
    pub inventory: PathBuf,
    /// Compatibility history.
    pub lock_file: PathBuf,
    /// SHA-256 of the normalized contract, shared by both endpoints.
    pub schema_hash: String,
}

/// Compile, validate, and generate one coherent Rust/TypeScript IPC contract.
///
/// Requires installed protoc and ts-proto 2.12.3. No tools are downloaded.
/// `check` reports drift without rewriting output or compatibility history.
pub fn generate(config: &GenerateConfig) -> Result<GeneratedFiles, GenerateError> {
    compiler::generate(config)
}
