// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::BTreeMap;

use serde::Serialize;

/// Native input/output inventory from the completed invocation, not an ESBuild metafile.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleMetadata {
    /// Canonical bundler module ID to exact captured source details.
    pub inputs: BTreeMap<String, InputMetadata>,
    /// Project-relative output path to emitted byte and retained-module details.
    pub outputs: BTreeMap<String, OutputMetadata>,
}

/// Exact bytes and original identity of one captured module.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputMetadata {
    /// Original source byte length (not generated loader code).
    pub bytes: usize,
    /// Project-relative physical identity or stable encoded virtual identity.
    pub source: String,
    /// Opaque non-JavaScript input; semantic analysis must not parse it.
    pub opaque: bool,
    /// Script grammar selected by the actual loader, absent for opaque inputs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub syntax: Option<super::ScriptSyntax>,
    /// Edges actually resolved by the native bundler.
    pub imports: Vec<MetadataImport>,
}

/// One native emitted output.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputMetadata {
    /// Final emitted bytes, after minification and charset conversion.
    pub bytes: usize,
    /// Canonical source module ID when this output is an entry facade.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_point: Option<String>,
    /// Exact static/dynamic dependencies; internal paths are project-relative.
    pub imports: Vec<MetadataImport>,
    /// Canonical retained module ID to Rolldown's real rendered contribution.
    pub inputs: BTreeMap<String, ModuleContribution>,
    /// Dynamic target to additional static-closure chunks whose import() calls
    /// start alongside it. Every path is project-relative; static edges remain.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub parallel_dynamic_imports: BTreeMap<String, Vec<String>>,
}

/// A real rendered-module contribution, explicitly not final minified attribution.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleContribution {
    /// UTF-8 bytes reported by Rolldown before chunk minification.
    pub rendered_bytes_before_minify: usize,
}

/// A resolved source or emitted output dependency.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataImport {
    /// Canonical target ID for source imports; project-relative for internal output imports.
    pub path: String,
    /// Static import or dynamic import.
    pub kind: ImportKind,
    /// Whether this dependency remains outside the emitted build.
    pub external: bool,
}

/// Import timing, shared by source and output inventories.
#[derive(Debug, Serialize)]
pub enum ImportKind {
    /// Eager/static dependency.
    #[serde(rename = "import-statement")]
    Static,
    /// Deferred dependency.
    #[serde(rename = "dynamic-import")]
    Dynamic,
}
