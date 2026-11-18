// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::Result;
use clap::Args;
use include_dir::{include_dir, Dir};

use crate::utils::output::{self, OutputFormat};

static ASSETS: Dir<'_> = include_dir!("$WEBUI_NATIVE_ASSETS");

pub(crate) fn license() -> Result<&'static [u8], NativeError> {
    ASSETS
        .get_file("LICENSE")
        .map(include_dir::File::contents)
        .ok_or_else(|| NativeError::Assets("missing bundled license notice".into()))
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum NativeError {
    #[error("invalid native application configuration: {0}")]
    Config(String),
    #[error("invalid native toolchain resources: {0}")]
    Assets(String),
    #[error(transparent)]
    Toolchain(#[from] super::toolchain::ToolchainError),
    #[error("native application host failed: {0}")]
    Host(String),
    #[error("native application command stopped")]
    Cancelled,
    #[error("native process cleanup failed: {0}")]
    Cleanup(String),
    #[error("could not update native metadata {}", path.display())]
    Metadata {
        path: PathBuf,
        #[source]
        source: anyhow::Error,
    },
    #[error("native application I/O failed at {}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Build(#[from] webui::browser_build::ClientBuildError),
}

impl NativeError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Config(_) => "native-invalid-config",
            Self::Assets(_) => "native-invalid-assets",
            Self::Toolchain(error) => error.code(),
            Self::Host(_) => "native-host-failed",
            Self::Cancelled => "native-command-stopped",
            Self::Cleanup(_) => "native-shutdown-failed",
            Self::Metadata { .. } => "native-io",
            Self::Io { .. } => "native-io",
            Self::Build(webui::browser_build::ClientBuildError::Projection(error)) => error
                .0
                .first()
                .map_or("native-client-build-failed", |diagnostic| diagnostic.code),
            Self::Build(_) => "native-client-build-failed",
        }
    }

    pub(crate) fn help(&self) -> &str {
        match self {
            Self::Config(_) => "Check webui.toml against the application configuration in the CLI reference.",
            Self::Assets(_) => "Use matching native WebUI tooling. Review conflicting files in .webui/toolchain before removing or restoring that version's cache; existing files are never silently replaced.",
            Self::Toolchain(error) => error.help(),
            Self::Host(_) => "Resolve the native compiler/server diagnostics, then retry the application command.",
            Self::Cancelled => "The requested stop was received.",
            Self::Cleanup(_) => "Inspect the native process diagnostics; the containing supervisor will terminate remaining owned processes.",
            Self::Metadata { .. } => "Check the reported application metadata path and filesystem permissions.",
            Self::Io { .. } => "Check the reported application path and filesystem permissions.",
            Self::Build(webui::browser_build::ClientBuildError::Projection(error)) =>
                error.0.first().map_or("Fix the unproven browser component declaration.", |diagnostic| diagnostic.help.as_str()),
            Self::Build(_) => "Fix the browser source or resource error. Native builds never fall back to a Node toolchain.",
        }
    }

    pub(crate) fn exit_code(&self) -> i32 {
        match self {
            Self::Io { .. } | Self::Metadata { .. } => 74,
            Self::Toolchain(error) => error.exit_code(),
            Self::Host(_) | Self::Cleanup(_) => 1,
            Self::Cancelled => 0,
            _ => 65,
        }
    }
}

#[derive(Args)]
pub(crate) struct ReferenceArgs {
    /// Materialize version-matched offline guides and print their core path
    #[arg(long)]
    path: bool,
    /// Application whose private toolchain cache owns the guides
    #[arg(long, default_value = ".")]
    project: PathBuf,
}

pub(crate) fn reference(args: &ReferenceArgs) -> Result<()> {
    let result = (|| {
        if args.path {
            let root = materialize(&args.project, "reference")?;
            let path = root.join("ai.md");
            if matches!(output::format(), OutputFormat::Json) {
                let mut value = serde_json::Map::new();
                value.insert(
                    "path".into(),
                    serde_json::Value::from(path.to_string_lossy().as_ref()),
                );
                value.insert(
                    "version".into(),
                    serde_json::Value::from(env!("CARGO_PKG_VERSION")),
                );
                println!("{}", serde_json::Value::Object(value));
            } else {
                println!("{}", path.display());
            }
        } else {
            let file = ASSETS
                .get_file("reference/ai.md")
                .and_then(|file| file.contents_utf8())
                .ok_or_else(|| NativeError::Assets("missing bundled core reference".into()))?;
            print!("{file}");
        }
        Ok(())
    })();
    result.inspect_err(output::error)
}

pub(crate) fn materialize(project: &Path, subtree: &str) -> Result<PathBuf, NativeError> {
    let root = fs::canonicalize(project).map_err(|source| io(project, source))?;
    let relative = Path::new(".webui/toolchain").join(env!("CARGO_PKG_VERSION"));
    ensure_directory(&root, &relative)?;
    let lock_path = root.join(&relative).join(".lock");
    let lock = open_lock(&lock_path)?;
    lock.lock().map_err(|source| io(&lock_path, source))?;
    let directory = ASSETS
        .get_dir(subtree)
        .ok_or_else(|| NativeError::Assets(format!("missing bundled {subtree} resources")))?;
    let mut pending = vec![directory];
    while let Some(directory) = pending.pop() {
        pending.extend(directory.dirs());
        for file in directory.files() {
            let path = relative.join(file.path());
            let parent = path
                .parent()
                .ok_or_else(|| NativeError::Assets("resource has no parent".into()))?;
            ensure_directory(&root, parent)?;
            write_resource(&root.join(path), file.contents())?;
        }
    }
    Ok(root.join(relative).join(subtree))
}

pub(crate) fn ensure_directory(root: &Path, relative: &Path) -> Result<(), NativeError> {
    let mut path = root.to_owned();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(NativeError::Assets(
                "resource path escaped its application cache".into(),
            ));
        };
        path.push(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(NativeError::Assets(format!(
                    "cache directory is redirected or not a directory: {}",
                    path.display()
                )))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&path).map_err(|source| io(&path, source))?
            }
            Err(source) => return Err(io(&path, source)),
        }
    }
    Ok(())
}

pub(crate) fn open_lock(path: &Path) -> Result<fs::File, NativeError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err(NativeError::Assets(format!(
                "lock is redirected or not a regular file: {}",
                path.display()
            )));
        }
        Err(source) if source.kind() != std::io::ErrorKind::NotFound => {
            return Err(io(path, source))
        }
        _ => {}
    }
    OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
        .map_err(|source| io(path, source))
}

fn write_resource(path: &Path, content: &[u8]) -> Result<(), NativeError> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => file.write_all(content).map_err(|source| io(path, source)),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(path).map_err(|source| io(path, source))?;
            if !metadata.file_type().is_file() || metadata.len() != content.len() as u64 {
                return Err(NativeError::Assets(format!(
                    "conflicting resource: {}",
                    path.display()
                )));
            }
            if fs::read(path).map_err(|source| io(path, source))? != content {
                return Err(NativeError::Assets(format!(
                    "modified resource: {}",
                    path.display()
                )));
            }
            Ok(())
        }
        Err(source) => Err(io(path, source)),
    }
}

pub(crate) fn io(path: &Path, source: std::io::Error) -> NativeError {
    NativeError::Io {
        path: path.to_owned(),
        source,
    }
}

pub(crate) fn read_metadata(path: &Path) -> Result<Vec<u8>, NativeError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io(path, source))?;
    if !metadata.file_type().is_file() {
        return Err(NativeError::Config(format!(
            "metadata must be a regular file: {}",
            path.display()
        )));
    }
    let file = fs::File::open(path).map_err(|source| io(path, source))?;
    let mut bytes = Vec::with_capacity(4096);
    file.take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|source| io(path, source))?;
    if bytes.len() > 65_536 {
        return Err(NativeError::Config(format!(
            "metadata exceeds 64 KiB: {}",
            path.display()
        )));
    }
    Ok(bytes)
}

pub(crate) fn write_metadata(
    root: &Path,
    relative: &Path,
    value: &impl serde::Serialize,
) -> Result<(), NativeError> {
    let parent = relative
        .parent()
        .ok_or_else(|| NativeError::Config("metadata path has no parent".into()))?;
    ensure_directory(root, parent)?;
    ensure_directory(root, &parent.join(".webui-metafile-tmp"))?;
    let path = root.join(relative);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err(NativeError::Assets(format!(
                "metadata path is redirected or not a file: {}",
                path.display()
            )))
        }
        Err(source) if source.kind() != std::io::ErrorKind::NotFound => {
            return Err(io(&path, source))
        }
        _ => {}
    }
    let json =
        serde_json::to_string(value).map_err(|error| NativeError::Config(error.to_string()))?;
    super::metafile::write_atomic(&path, &json)
        .map_err(|source| NativeError::Metadata { path, source })
}
pub(crate) mod control;
