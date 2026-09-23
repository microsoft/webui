// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::{
    error::{io, schema},
    GenerateError,
};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

// protoc 29.x canonicalizes backslashes to slashes, corrupting a verbatim
// \\?\ prefix before its Windows file opener sees it. Keep canonical paths for
// our filesystem/security checks; adapt only the paths handed to external tools.
pub(super) fn protoc_path(path: &Path) -> Result<PathBuf, GenerateError> {
    let text = path.to_str().ok_or_else(|| {
        schema(
            "ipc-tool-path",
            path.display().to_string(),
            "protoc paths must be valid Unicode",
            "use Unicode schema, output, and compiler paths",
        )
    })?;
    let Some(ordinary) = text.strip_prefix(r"\\?\") else {
        return Ok(path.to_owned());
    };
    let bytes = ordinary.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\' {
        return Ok(PathBuf::from(ordinary));
    }
    Err(schema(
        "ipc-tool-path",
        text,
        "protoc does not support this Windows UNC/device namespace",
        "generate from a local drive path; keep UNC/device paths out of protoc inputs and outputs",
    ))
}

pub(super) fn program(path: &Path) -> Result<PathBuf, GenerateError> {
    if path.components().count() > 1 {
        let absolute = std::path::absolute(path).map_err(|e| io(path, e))?;
        protoc_path(&absolute)
    } else {
        if let Some(search) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&search) {
                let mut candidate = directory.join(path);
                if cfg!(windows) && candidate.extension().is_none() {
                    candidate.set_extension("exe");
                }
                if candidate.is_file() {
                    let canonical =
                        std::fs::canonicalize(&candidate).map_err(|e| io(&candidate, e))?;
                    return protoc_path(&canonical);
                }
            }
        }
        Ok(path.to_owned())
    }
}

pub(super) fn path_option(
    command: &mut Command,
    option: &str,
    path: &Path,
) -> Result<(), GenerateError> {
    let absolute = std::path::absolute(path).map_err(|e| io(path, e))?;
    let mut argument = OsString::from(option);
    argument.push(protoc_path(&absolute)?);
    command.arg(argument);
    Ok(())
}
