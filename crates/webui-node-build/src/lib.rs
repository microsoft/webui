// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Build-only Node.js bindings for WebUI compilation and native client bundling.
//!
//! The runtime addon consumes the owned protocol buffers returned here without
//! loading this compiler addon or sharing opaque native handles.

use napi::bindgen_prelude::Buffer;
use napi::Error as NapiError;
use napi_derive::napi;

mod client_build;
mod prepared_build;
mod types;

pub use client_build::bundle_client;
pub use prepared_build::{prepare_build, JsFinishBuildOptions, JsPreparedBuild};
pub use types::{
    JsBuildOptions, JsBuildResult, JsBuildStats, JsGeneratedDeclaration, JsGeneratedModule,
    JsLazyPackFile, JsProjectionManifest,
};

/// Build a WebUI application from an app directory.
///
/// Returns the compiled protocol bytes, CSS files, and build statistics.
#[napi]
pub fn build(options: JsBuildOptions) -> napi::Result<JsBuildResult> {
    let result = webui::build(build_options(options)?)
        .map_err(|e| NapiError::from_reason(format!("Build error: {}", e.chain_message())))?;
    Ok(js_build_result(result))
}

fn build_options(options: JsBuildOptions) -> napi::Result<webui::BuildOptions> {
    let css = options
        .css
        .map(|s| s.parse::<webui::CssStrategy>())
        .transpose()
        .map_err(NapiError::from_reason)?
        .unwrap_or_default();
    let dom = options
        .dom
        .map(|value| value.parse::<webui::DomStrategy>())
        .transpose()
        .map_err(NapiError::from_reason)?
        .unwrap_or_default();

    let plugin = options
        .plugin
        .map(|s| s.parse::<webui::Plugin>())
        .transpose()
        .map_err(NapiError::from_reason)?;

    let legal_comments = options
        .legal_comments
        .map(|s| s.parse::<webui::LegalComments>())
        .transpose()
        .map_err(NapiError::from_reason)?
        .unwrap_or_default();

    let app_dir = std::path::PathBuf::from(&options.app_dir);
    let theme = options
        .theme
        .as_deref()
        .map(|theme| load_theme(theme, &app_dir))
        .transpose()?;
    let projection_manifests = projection_sources(
        options.projection_manifests,
        options.projection_manifest_objects,
    );

    let build_options = webui::BuildOptions {
        app_dir,
        entry: options.entry.unwrap_or_else(|| "index.html".to_string()),
        css,
        dom,
        css_bundle: options.css_bundle.unwrap_or(false),
        plugin,
        components: options.components.unwrap_or_default(),
        component_asset_roots: options.component_asset_roots.unwrap_or_default(),
        metafile: options.metafile.unwrap_or(false),
        css_file_name_template: options
            .css_file_name_template
            .unwrap_or_else(|| webui::DEFAULT_CSS_FILE_NAME_TEMPLATE.to_string()),
        asset_base: options.asset_base,
        component_modules: options
            .component_modules
            .unwrap_or_default()
            .into_iter()
            .collect(),
        modules_out: options.modules_out.map(std::path::PathBuf::from),
        legal_comments,
        theme,
        projection_manifests,
    };

    Ok(build_options)
}

fn projection_sources(
    paths: Option<Vec<String>>,
    objects: Option<Vec<JsProjectionManifest>>,
) -> Vec<webui::ProjectionManifestSource> {
    let mut sources: Vec<webui::ProjectionManifestSource> = paths
        .unwrap_or_default()
        .into_iter()
        .map(std::path::PathBuf::from)
        .map(Into::into)
        .collect();
    sources.extend(objects.unwrap_or_default().into_iter().map(|manifest| {
        webui::ProjectionManifestSource::Inline {
            manifest_path: manifest.path.into(),
            json: manifest.json,
        }
    }));
    sources
}

#[allow(clippy::cast_possible_truncation)] // stats are bounded by component/file counts
fn js_build_result(result: webui::BuildResult) -> JsBuildResult {
    // Flatten css_files into alternating [filename, content, ...] for JS interop
    let css_files: Vec<String> = result
        .css_files
        .into_iter()
        .flat_map(|(name, content)| [name, content])
        .collect();
    let component_asset_files: Vec<String> = result
        .component_asset_files
        .into_iter()
        .flat_map(|file| [file.name, file.content])
        .collect();
    let lazy_pack_files: Vec<JsLazyPackFile> = result
        .lazy_pack_files
        .into_iter()
        .map(js_lazy_pack_file)
        .collect();
    let warnings: Vec<String> = result.warnings.iter().map(|d| d.to_string()).collect();

    JsBuildResult {
        protocol: Buffer::from(result.protocol_bytes),
        css_files,
        component_asset_files,
        lazy_pack_files,
        lazy_scripts_module: result.lazy_scripts_module.map(|module| JsGeneratedModule {
            file_name: module.file_name,
            content: module.content,
            declaration: JsGeneratedDeclaration {
                file_name: module.declaration.file_name,
                content: module.declaration.content,
            },
        }),
        metafile: result.metafile,
        warnings,
        stats: JsBuildStats {
            duration_ms: result.stats.duration.as_secs_f64() * 1000.0,
            fragment_count: result.stats.fragment_count as u32,
            component_count: result.stats.component_count as u32,
            css_file_count: result.stats.css_file_count as u32,
            protocol_size_bytes: result.stats.protocol_size_bytes as u32,
            token_count: result.stats.token_count as u32,
        },
    }
}

fn js_lazy_pack_file(pack: webui::LazyPackFile) -> JsLazyPackFile {
    let path = pack.path();
    let hash = pack.hash().to_string();
    let to_u32 = |bytes: usize| u32::try_from(bytes).unwrap_or(u32::MAX);
    JsLazyPackFile {
        bytes: to_u32(pack.bytes()),
        brotli_bytes: pack.brotli_bytes.map(to_u32),
        scripts: pack
            .scripts
            .into_iter()
            .map(|script| script.specifier)
            .collect(),
        stylesheets: pack.stylesheets,
        trigger: pack.trigger.as_str().to_string(),
        file_name: pack.file_name,
        content: pack.content,
        path,
        region: pack.region,
        owner: pack.owner,
        hash,
    }
}

fn load_theme(theme: &str, search_root: &std::path::Path) -> napi::Result<webui::TokenFile> {
    let resolved = webui::resolve_theme_path(theme, search_root)
        .map_err(|e| NapiError::from_reason(format!("Theme resolution error: {e}")))?;
    webui::load_token_file(&resolved).map_err(|e| {
        NapiError::from_reason(format!("Theme load error for {}: {e}", resolved.display()))
    })
}

#[cfg(test)]
mod tests;
