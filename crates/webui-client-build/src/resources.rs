// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use crate::ClientBuildError;

pub(super) type FrameworkAliases = Vec<(String, Vec<Option<String>>)>;

pub(super) fn input_key(root: &Path, id: &str) -> Result<String, ClientBuildError> {
    if let Some(id) = id.strip_prefix('\0') {
        return Ok(crate::manifest::hex("virtual:", id.as_bytes()));
    }
    let relative = Path::new(id).strip_prefix(root).map_err(|_| {
        ClientBuildError::Invalid("captured input escaped its project boundary".into())
    })?;
    if relative.as_os_str().is_empty() {
        return Err(ClientBuildError::Invalid(
            "input must identify a file".into(),
        ));
    }
    Ok(portable(relative))
}

pub(super) fn portable(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(super) fn open_bounded(
    path: &Path,
    limit: usize,
) -> Result<(std::fs::File, usize), ClientBuildError> {
    // Reject special files before opening (a FIFO can otherwise block forever).
    let metadata = std::fs::metadata(path).map_err(|source| ClientBuildError::Read {
        path: path.to_owned(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(ClientBuildError::Invalid(
            "browser inputs must be regular files; replace the special input file".into(),
        ));
    }
    // Check again on the opened handle; reads remain bounded if the file grows.
    let file = std::fs::File::open(path).map_err(|source| ClientBuildError::Read {
        path: path.to_owned(),
        source,
    })?;
    let metadata = file.metadata().map_err(|source| ClientBuildError::Read {
        path: path.to_owned(),
        source,
    })?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err(ClientBuildError::Invalid(format!(
            "browser input {} must be a regular file no larger than {} bytes (16 MiB/module maximum)",
            path.display(), limit
        )));
    }
    let size = usize::try_from(metadata.len())
        .map_err(|error| ClientBuildError::Invalid(error.to_string()))?;
    Ok((file, size))
}

pub(super) fn read_bounded(
    file: std::fs::File,
    path: &Path,
    limit: usize,
) -> Result<Vec<u8>, ClientBuildError> {
    let mut bytes = Vec::with_capacity(limit.min(65_536));
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| ClientBuildError::Read {
            path: path.to_owned(),
            source,
        })?;
    if bytes.len() > limit {
        return Err(ClientBuildError::Invalid(
            "browser input grew beyond its bounded read limit (16 MiB/module maximum)".into(),
        ));
    }
    Ok(bytes)
}

pub(super) fn utf8(path: &Path) -> Result<&str, ClientBuildError> {
    path.to_str()
        .ok_or_else(|| ClientBuildError::Invalid("browser build paths must be UTF-8".into()))
}

pub(super) fn contained(root: &Path, path: &Path) -> Result<PathBuf, ClientBuildError> {
    let path = root.join(path);
    let resolved =
        std::fs::canonicalize(&path).map_err(|source| ClientBuildError::Read { path, source })?;
    if !resolved.starts_with(root) {
        return Err(ClientBuildError::Invalid(
            "physical browser inputs must remain inside the project".into(),
        ));
    }
    Ok(resolved)
}

pub(super) fn output_path(root: &Path, path: &Path) -> Result<PathBuf, ClientBuildError> {
    if path.as_os_str().is_empty()
        || utf8(path)?.contains(['\0', ':'])
        || (!cfg!(windows) && utf8(path)?.contains('\\'))
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(ClientBuildError::Invalid(
            "browser output must be a nonempty project-relative path without parent segments"
                .into(),
        ));
    }
    Ok(root.join(path))
}

pub(super) fn relative_output(
    root: &Path,
    out: &Path,
    filename: &str,
) -> Result<PathBuf, ClientBuildError> {
    let path = output_path(out, Path::new(filename))?;
    path.strip_prefix(root)
        .map(Path::to_owned)
        .map_err(|_| ClientBuildError::Invalid("browser output escaped the project".into()))
}

pub(super) fn framework_exports(root: &Path) -> Result<FrameworkAliases, ClientBuildError> {
    let path = contained(root, Path::new("package.json"))?;
    let (file, _) = open_bounded(&path, 65_536)?;
    let bytes = read_bounded(file, &path, 65_536)?;
    let metadata: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| ClientBuildError::Invalid(error.to_string()))?;
    if metadata["name"] != "@microsoft/webui-framework"
        || metadata["version"] != env!("CARGO_PKG_VERSION")
    {
        return Err(ClientBuildError::Invalid(
            "framework resources must match the WebUI compiler release".into(),
        ));
    }
    let exports = metadata["exports"].as_object().ok_or_else(|| {
        ClientBuildError::Invalid("framework resources are missing their export map".into())
    })?;
    let mut aliases = Vec::with_capacity(exports.len());
    for (specifier, value) in exports {
        let target = value["import"]["default"].as_str().ok_or_else(|| {
            ClientBuildError::Invalid(format!("framework export {specifier} has no ESM target"))
        })?;
        let suffix = if specifier == "." {
            ""
        } else {
            specifier
                .strip_prefix('.')
                .filter(|suffix| suffix.starts_with('/'))
                .ok_or_else(|| {
                    ClientBuildError::Invalid("invalid framework export specifier".into())
                })?
        };
        let resolved = contained(root, Path::new(target))?;
        aliases.push((
            format!("@microsoft/webui-framework{suffix}$"),
            vec![Some(utf8(&resolved)?.to_owned())],
        ));
    }
    Ok(aliases)
}
