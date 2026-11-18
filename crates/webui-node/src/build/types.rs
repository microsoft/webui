// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Owned JavaScript values returned by or supplied to the compiler addon.

use std::collections::HashMap;

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;

/// Build statistics returned from the build function.
#[napi(object)]
pub struct JsBuildStats {
    /// Build duration in milliseconds.
    pub duration_ms: f64,
    /// Total number of protocol fragments.
    pub fragment_count: u32,
    /// Number of registered components.
    pub component_count: u32,
    /// Number of CSS files produced.
    pub css_file_count: u32,
    /// Size of the serialized protocol in bytes.
    pub protocol_size_bytes: u32,
    /// Number of unique CSS tokens discovered.
    pub token_count: u32,
}

/// Result of a successful build operation.
#[napi(object)]
pub struct JsBuildResult {
    /// Serialized protocol (protobuf binary).
    pub protocol: Buffer,
    /// CSS files as alternating [filename, content, filename, content, ...].
    pub css_files: Vec<String>,
    /// Static component asset files as alternating [filename, content, filename, content, ...].
    pub component_asset_files: Vec<String>,
    /// Lazy-region packs; publish each under `_webui/packs/` in the build output.
    pub lazy_pack_files: Vec<JsLazyPackFile>,
    /// Generated `lazy.webui.js` and its `lazy.webui.d.ts`, present when the
    /// build has lazy regions.
    pub lazy_scripts_module: Option<JsGeneratedModule>,
    /// Build metafile JSON when requested and the build emits component assets
    /// or lazy packs.
    pub metafile: Option<String>,
    /// Non-fatal build advisories (plain text), e.g. CSS tokens used only with a
    /// literal `var()` fallback and absent from every theme.
    pub warnings: Vec<String>,
    /// Build statistics.
    pub stats: JsBuildStats,
}

/// One lazy-region template pack and its cost report.
#[napi(object)]
pub struct JsLazyPackFile {
    /// Content-addressed file name inside `_webui/packs/`.
    pub file_name: String,
    /// JSON pack content.
    pub content: String,
    /// Path relative to the asset base: `_webui/packs/<fileName>`.
    pub path: String,
    /// Region identity, `<owner>::lazy:@<n>`.
    pub region: String,
    /// Component whose template declares the region.
    pub owner: String,
    /// Prefetch trigger: `intent`, `idle`, `eager`, or `demand`.
    pub trigger: String,
    /// Content hash from the file name.
    pub hash: String,
    /// Size of `content` in bytes.
    pub bytes: u32,
    /// Brotli size at quality 11, present when `metafile` is requested.
    pub brotli_bytes: Option<u32>,
    /// Module specifiers of the scripted components the region introduces.
    pub scripts: Vec<String>,
    /// Stylesheet paths, relative to the asset base, that the pack registers
    /// in a Link build; empty in Style and Module builds.
    pub stylesheets: Vec<String>,
}

/// A generated ES module for the application bundler.
#[napi(object)]
pub struct JsGeneratedModule {
    /// File name to write in the generated-modules directory.
    pub file_name: String,
    /// JavaScript module source.
    pub content: String,
    /// TypeScript declarations to write next to the module.
    pub declaration: JsGeneratedDeclaration,
}

/// TypeScript declarations for a generated module, such as `lazy.webui.d.ts`.
#[napi(object)]
pub struct JsGeneratedDeclaration {
    /// File name to write in the generated-modules directory.
    pub file_name: String,
    /// TypeScript declaration source.
    pub content: String,
}

/// Inline projection manifest transported through N-API.
#[napi(object)]
pub struct JsProjectionManifest {
    /// Logical manifest path used to resolve `root` and stale file checks.
    pub path: String,
    /// Canonical manifest JSON.
    pub json: String,
}

/// Build options for the webui build API.
#[napi(object)]
pub struct JsBuildOptions {
    /// Path to the application folder containing templates.
    pub app_dir: String,
    /// Entry HTML file name (defaults to "index.html").
    pub entry: Option<String>,
    /// CSS mode: "link" (default), "style", or "module".
    pub css: Option<String>,
    /// Fallback DOM strategy: "shadow" (default) or "light".
    pub dom: Option<String>,
    /// Merge component stylesheets into shared bundled chunks.
    ///
    /// Composes with `css`: bundling decides how stylesheets are grouped, `css`
    /// decides how they reach the page.
    pub css_bundle: Option<bool>,
    /// Plugin identifier: "webui", "fast-v3", "fast-v2", or "fast".
    pub plugin: Option<String>,
    /// Additional component sources (npm packages or local paths).
    pub components: Option<Vec<String>>,
    /// Root component tags emitted as static `.webui.js` ESM assets.
    pub component_asset_roots: Option<Vec<String>>,
    /// Generate and return the build metafile: the component asset graph and
    /// the lazy-region pack report.
    pub metafile: Option<bool>,
    /// Link-mode CSS filename template using [name], [hash], [ext].
    pub css_file_name_template: Option<String>,
    /// Public URL or path ending in "/" that maps to the output directory.
    /// Requests override it with the `$webui.assetBase` state member.
    pub asset_base: Option<String>,
    /// Module specifiers for custom elements without a usable sibling module,
    /// keyed by tag; written verbatim in `lazy.webui.js`.
    pub component_modules: Option<HashMap<String, String>>,
    /// Directory the generated modules will be written to. Relative module
    /// specifiers in `lazy.webui.js` start here; defaults to `appDir`.
    pub modules_out: Option<String>,
    /// Legal comment handling: "inline" (default) or "none".
    pub legal_comments: Option<String>,
    /// Design token theme: a JSON file path or npm package name.
    pub theme: Option<String>,
    /// Projection manifest paths.
    pub projection_manifests: Option<Vec<String>>,
    /// Inline manifest objects with their logical paths.
    pub projection_manifest_objects: Option<Vec<JsProjectionManifest>>,
}
