// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Source map publication mode.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceMap {
    /// Do not generate maps.
    #[default]
    None,
    /// Emit a map file and a sourceMappingURL.
    File,
    /// Embed the map in the JavaScript.
    Inline,
    /// Emit a map file without a sourceMappingURL.
    Hidden,
}

/// Browser JavaScript output format.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// ECMAScript modules, with code splitting.
    #[default]
    Esm,
    /// Self-executing side-effect script, without an export namespace.
    /// Dynamic dependencies are inlined.
    Iife,
}

/// Explicit extension loader. CSS imports with `type: "css"` produce CSSStyleSheets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Loader {
    /// JavaScript.
    Js,
    /// TypeScript.
    Ts,
    /// TypeScript with JSX.
    Tsx,
    /// JavaScript with JSX.
    Jsx,
    /// JSON default and named exports.
    Json,
    /// UTF-8 string default export.
    Text,
    /// Reserved: stylesheet bundling is not supported by this Rolldown release.
    Css,
    /// Emit opaque bytes as an asset and export its URL.
    File,
    /// Export opaque bytes as an inline data URL.
    DataUrl,
}

/// One bounded native browser build. All physical inputs must remain inside `project`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClientBuildOptions {
    /// Explicit boundary containing all physical browser inputs.
    pub project: PathBuf,
    /// Entry name (possibly nested) to project-relative source or exact NUL virtual ID.
    pub entries: BTreeMap<String, PathBuf>,
    /// Project-relative destination; the builder never writes it.
    pub out_dir: PathBuf,
    /// Optional extracted, release-matching framework package.
    #[serde(default)]
    pub framework: Option<PathBuf>,
    /// Value of the canonical `__WEBUI_DEV__` compile-time define.
    #[serde(default)]
    pub development: bool,
    /// Minify JavaScript (independent of development policy).
    #[serde(default = "yes")]
    pub minify: bool,
    /// Source map mode.
    #[serde(default)]
    pub sourcemap: SourceMap,
    /// JavaScript output format.
    #[serde(default)]
    pub format: Format,
    /// Native transformation target.
    #[serde(default = "target")]
    pub target: String,
    /// Literal compile-time replacements understood by Rolldown.
    #[serde(default)]
    pub defines: BTreeMap<String, String>,
    /// Native package/path aliases; an optional trailing `$` requests exact matching.
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
    /// External module IDs. Bare package IDs also cover `/subpaths`; aliases win.
    #[serde(default)]
    pub external: Vec<String>,
    /// Rolldown entry filename template, including extension.
    #[serde(default = "entry_names")]
    pub entry_names: String,
    /// Rolldown shared/dynamic chunk filename template, including extension.
    #[serde(default = "chunk_names")]
    pub chunk_names: String,
    /// URL prefix for emitted chunks and assets.
    #[serde(default)]
    pub public_path: Option<String>,
    /// Start each internal dynamic target and its unavailable static closure in
    /// one Promise-all import group. This opt-in may evaluate shared dependencies
    /// independently, matching explicit parallel dynamic-import scheduling.
    #[serde(default)]
    pub parallel_dynamic_imports: bool,
    /// Extension (including `.`) to explicit loader.
    #[serde(default)]
    pub loaders: BTreeMap<String, Loader>,
    /// Stable, exact NUL-prefixed module IDs to authored JavaScript/TypeScript.
    #[serde(default)]
    pub virtual_modules: BTreeMap<String, String>,
    /// Optional explicit native tsconfig path inside the project.
    #[serde(default)]
    pub tsconfig: Option<PathBuf>,
}

impl Default for ClientBuildOptions {
    fn default() -> Self {
        Self {
            project: PathBuf::new(),
            entries: BTreeMap::new(),
            out_dir: PathBuf::new(),
            framework: None,
            development: false,
            minify: true,
            sourcemap: SourceMap::None,
            format: Format::Esm,
            target: target(),
            defines: BTreeMap::new(),
            aliases: BTreeMap::new(),
            external: Vec::new(),
            entry_names: entry_names(),
            chunk_names: chunk_names(),
            public_path: None,
            parallel_dynamic_imports: false,
            loaders: BTreeMap::new(),
            virtual_modules: BTreeMap::new(),
            tsconfig: None,
        }
    }
}

fn yes() -> bool {
    true
}

fn target() -> String {
    "es2022".into()
}

fn entry_names() -> String {
    "[name].js".into()
}

fn chunk_names() -> String {
    "[name]-[hash].js".into()
}
