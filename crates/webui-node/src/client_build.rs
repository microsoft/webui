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
        // std canonicalization uses verbatim Windows prefixes, while Node paths
        // use ordinary drive/UNC prefixes. Compare components without resolving
        // the (possibly nonexistent) output or normalizing away parent segments.
        let mut parts = path.components();
        for expected in project.components() {
            if !parts
                .next()
                .is_some_and(|part| same_component(expected, part))
            {
                return Err(Error::from_reason(
                    "Native browser outputs must remain inside the canonical project",
                ));
            }
        }
        parts.as_path()
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

fn same_component(left: Component<'_>, right: Component<'_>) -> bool {
    #[cfg(windows)]
    if let (Component::Prefix(left), Component::Prefix(right)) = (left, right) {
        use std::path::Prefix;
        return match (left.kind(), right.kind()) {
            (Prefix::VerbatimDisk(a), Prefix::Disk(b))
            | (Prefix::Disk(a), Prefix::VerbatimDisk(b)) => a == b,
            (Prefix::VerbatimUNC(a, b), Prefix::UNC(c, d))
            | (Prefix::UNC(a, b), Prefix::VerbatimUNC(c, d)) => a == c && b == d,
            (a, b) => a == b,
        };
    }
    left == right
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

#[cfg(all(test, windows))]
mod tests {
    use super::relative_path;
    use std::path::{Path, PathBuf};

    #[test]
    fn windows_output_prefixes_preserve_project_containment() -> napi::Result<()> {
        for (root, output) in [
            (r"\\?\C:\project", r"C:\project\dist\index.js"),
            (r"\\?\C:\project", r"C:/project/dist/index.js"),
            (r"\\?\C:\project", r"\\?\C:\project\dist\index.js"),
            (
                r"\\?\UNC\server\share\project",
                r"\\server\share\project\dist\index.js",
            ),
        ] {
            assert_eq!(
                relative_path(Path::new(root), Path::new(output))?,
                PathBuf::from(r"dist\index.js")
            );
        }
        for output in [
            r"D:\project\dist\index.js",
            r"C:\project-other\dist\index.js",
            r"C:\project\dist\..\index.js",
            r"C:\project",
        ] {
            assert!(relative_path(Path::new(r"\\?\C:\project"), Path::new(output)).is_err());
        }
        assert!(relative_path(
            Path::new(r"\\?\UNC\server\share\project"),
            Path::new(r"\\server\other\project\dist\index.js")
        )
        .is_err());
        Ok(())
    }
}
