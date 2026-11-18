// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use napi::Error;
use napi_derive::napi;

use crate::{
    build_options, js_build_result, projection_sources, JsBuildOptions, JsBuildResult,
    JsGeneratedModule,
};

/// Completed client manifests for a retained compilation.
#[napi(object)]
pub struct JsFinishBuildOptions {
    /// Filesystem paths of completed projection manifests.
    pub projection_manifests: Option<Vec<String>>,
    /// Inline manifests with their logical filesystem paths.
    pub projection_manifest_objects: Option<Vec<crate::JsProjectionManifest>>,
}

/// A single-use native compilation retained during client bundling.
#[napi(js_name = "PreparedBuild")]
pub struct JsPreparedBuild {
    inner: Option<webui::PreparedBuild>,
}

/// Compile the application once before bundling its generated client inputs.
#[napi]
pub fn prepare_build(options: JsBuildOptions) -> napi::Result<JsPreparedBuild> {
    let inner = webui::prepare_build(build_options(options)?)
        .map_err(|error| Error::from_reason(error.chain_message()))?;
    Ok(JsPreparedBuild { inner: Some(inner) })
}

#[napi]
impl JsPreparedBuild {
    /// Generated lazy behavior imports, when the application contains lazy regions.
    #[napi(getter)]
    pub fn lazy_scripts_module(&self) -> napi::Result<Option<JsGeneratedModule>> {
        Ok(self
            .active()?
            .lazy_scripts_module()
            .map(|module| JsGeneratedModule {
                file_name: module.file_name.clone(),
                content: module.content.clone(),
                declaration: crate::JsGeneratedDeclaration {
                    file_name: module.declaration.file_name.clone(),
                    content: module.declaration.content.clone(),
                },
            }))
    }

    /// Component modules as alternating [filename, content, ...] bundler inputs.
    #[napi(getter)]
    pub fn component_asset_files(&self) -> napi::Result<Vec<String>> {
        Ok(self
            .active()?
            .component_asset_files()
            .iter()
            .flat_map(|file| [file.name.clone(), file.content.clone()])
            .collect())
    }

    /// Projection-independent template packs and their region metadata.
    #[napi(getter)]
    pub fn lazy_pack_files(&self) -> napi::Result<Vec<crate::JsLazyPackFile>> {
        Ok(self
            .active()?
            .lazy_pack_files()
            .iter()
            .cloned()
            .map(crate::js_lazy_pack_file)
            .collect())
    }

    /// Publish generated bundler inputs using the compiler's transactional writer.
    #[napi]
    pub fn write_modules(&self) -> napi::Result<()> {
        self.active()?
            .write_modules()
            .map_err(|error| Error::from_reason(error.chain_message()))
    }

    /// Consume this compilation, validate the client manifests, and serialize it.
    #[napi]
    pub fn finish(&mut self, options: JsFinishBuildOptions) -> napi::Result<JsBuildResult> {
        let sources = projection_sources(
            options.projection_manifests,
            options.projection_manifest_objects,
        );
        self.inner
            .take()
            .ok_or_else(finished)?
            .finish(sources)
            .map(js_build_result)
            .map_err(|error| Error::from_reason(error.chain_message()))
    }
}

impl JsPreparedBuild {
    fn active(&self) -> napi::Result<&webui::PreparedBuild> {
        self.inner.as_ref().ok_or_else(finished)
    }
}

#[cold]
#[inline(never)]
fn finished() -> Error {
    Error::from_reason(
        "Prepared build has already been consumed. Call prepareBuild() for a new compilation.",
    )
}
