// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::{Component, Path, PathBuf};

use microsoft_webui_client_build::{build, BuildWarning, ClientBuildOptions};
use napi::bindgen_prelude::{AsyncTask, Buffer};
use napi::{Env, Error, Task};
use napi_derive::napi;

/// One final native browser output, with its absolute destination.
#[napi(object)]
pub struct JsClientFile {
    /// Validated absolute output path.
    pub path: String,
    /// Final bytes used by projection hashing.
    pub contents: Buffer,
}

/// Native browser build data transported without a JavaScript graph adapter.
#[napi(object)]
pub struct JsClientBuild {
    /// Final emitted files.
    pub files: Vec<JsClientFile>,
    /// Absolute location anchoring the native proof.
    pub manifest_path: String,
    /// Canonical native projection manifest.
    pub manifest_json: String,
    /// Native bundler input/output metadata.
    pub metafile_json: String,
    /// Explicit nonfatal native compiler advisories.
    pub warnings: Vec<JsClientBuildWarning>,
}

/// One actionable advisory from the native compiler.
#[napi(object)]
pub struct JsClientBuildWarning {
    /// Stable native warning code.
    pub code: String,
    /// Color-free diagnostic explanation.
    pub message: String,
}

/// A browser build executed on the native worker pool, not the Node event loop.
pub struct ClientBuildTask {
    options: Option<String>,
    manifest: Option<String>,
    write: bool,
}

/// Owned buffers released or transferred when one native build completes.
pub struct ClientBuildOutput {
    files: Vec<(String, Vec<u8>)>,
    manifest_path: String,
    manifest_json: String,
    metafile_json: String,
    warnings: Vec<BuildWarning>,
}

impl Task for ClientBuildTask {
    type Output = ClientBuildOutput;
    type JsValue = JsClientBuild;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        let json = self
            .options
            .take()
            .ok_or_else(|| Error::from_reason("Native client build has already been consumed"))?;
        let mut options: ClientBuildOptions = serde_json::from_str(&json).map_err(bundle_error)?;
        let project = fs::canonicalize(&options.project).map_err(bundle_error)?;
        options.out_dir = relative_path(&project, &options.out_dir)?;
        let manifest = self
            .manifest
            .take()
            .map(PathBuf::from)
            .unwrap_or_else(|| options.out_dir.join("webui-projection.json"));
        let relative_manifest = relative_path(&project, &manifest)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(bundle_error)?;
        let client = runtime.block_on(build(options)).map_err(bundle_error)?;
        if client.files.contains_key(&relative_manifest) {
            return Err(Error::from_reason(
                "The native projection manifest cannot replace a browser output",
            ));
        }
        let manifest = if self.write {
            client.write_outputs(&relative_manifest)
        } else {
            client.projection_manifest(&relative_manifest)
        }
        .map_err(bundle_error)?;
        let manifest_json = serde_json::to_string(&manifest).map_err(bundle_error)?;
        let metafile_json = serde_json::to_string(&client.metadata).map_err(bundle_error)?;
        let manifest_path = project.join(&relative_manifest);
        let mut files = Vec::with_capacity(client.files.len());
        for (relative, bytes) in client.files {
            let relative = relative_path(&project, &relative)?;
            files.push((utf8(&project.join(relative))?, bytes));
        }
        Ok(ClientBuildOutput {
            files,
            manifest_path: utf8(&manifest_path)?,
            manifest_json,
            metafile_json,
            warnings: client.warnings,
        })
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(JsClientBuild {
            files: output
                .files
                .into_iter()
                .map(|(path, bytes)| JsClientFile {
                    path,
                    contents: Buffer::from(bytes),
                })
                .collect(),
            manifest_path: output.manifest_path,
            manifest_json: output.manifest_json,
            metafile_json: output.metafile_json,
            warnings: output
                .warnings
                .into_iter()
                .map(|warning| JsClientBuildWarning {
                    code: warning.code,
                    message: warning.message,
                })
                .collect(),
        })
    }
}

/// Run Rolldown, source capture, semantic projection and final-byte hashing natively.
#[napi(js_name = "bundleClient")]
pub fn bundle_client(
    options_json: String,
    manifest_path: Option<String>,
    write: Option<bool>,
) -> AsyncTask<ClientBuildTask> {
    AsyncTask::new(ClientBuildTask {
        options: Some(options_json),
        manifest: manifest_path,
        write: write.unwrap_or(true),
    })
}

fn relative_path(project: &Path, path: &Path) -> napi::Result<PathBuf> {
    let relative = if path.is_absolute() {
        path.strip_prefix(project).map_err(bundle_error)?
    } else {
        path
    };
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(Error::from_reason(
            "Native browser outputs must be nonempty paths inside the project without parent segments",
        ));
    }
    Ok(relative.to_path_buf())
}

fn utf8(path: &Path) -> napi::Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::from_reason("Native browser build paths must be UTF-8"))
}

#[cold]
#[inline(never)]
fn bundle_error(error: impl std::fmt::Display) -> Error {
    Error::from_reason(format!("Native browser build failed: {error}"))
}
