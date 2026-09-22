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

pub(super) fn configure_plugin(
    command: &mut Command,
    plugin: &Path,
    workdir: &Path,
) -> Result<(), GenerateError> {
    let batch = plugin
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    if cfg!(windows) && batch {
        windows_shim(command, plugin, workdir)
    } else {
        path_option(command, "--plugin=protoc-gen-ts_proto=", plugin)
    }
}

pub(super) fn windows_shim(
    command: &mut Command,
    plugin: &Path,
    workdir: &Path,
) -> Result<(), GenerateError> {
    let extension = plugin
        .extension()
        .and_then(|e| e.to_str())
        .ok_or_else(|| shim_error(plugin))?;
    if !plugin
        .file_stem()
        .is_some_and(|s| s.eq_ignore_ascii_case("protoc-gen-ts_proto"))
        || !["cmd", "bat"]
            .iter()
            .any(|e| extension.eq_ignore_ascii_case(e))
    {
        return Err(shim_error(plugin));
    }
    let parent = plugin.parent().ok_or_else(|| shim_error(plugin))?;
    // Explicit --plugin mappings use CreateProcessW(EXACT_NAME), which cannot
    // execute npm .cmd shims. Protoc's SEARCH_PATH mode uses cmd.exe instead.
    // Isolate its CWD and put the verified shim first without mutating process CWD/PATH.
    for other in ["com", "exe", "bat", "cmd"] {
        if !extension.eq_ignore_ascii_case(other)
            && parent.join(format!("protoc-gen-ts_proto.{other}")).exists()
        {
            return Err(schema("ipc-plugin-shadow", plugin.display().to_string(),
                "another executable could shadow the selected ts-proto batch shim",
                "use an installation directory containing only the selected protoc-gen-ts_proto executable shim"));
        }
    }
    let mut paths = vec![protoc_path(parent)?];
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path));
    }
    let path = std::env::join_paths(paths).map_err(|e| {
        schema(
            "ipc-tool-path",
            plugin.display().to_string(),
            e.to_string(),
            "use a ts-proto installation path representable in PATH",
        )
    })?;
    // Preserve .EXE lookup inside npm's shim so it can invoke node.exe.
    command
        .current_dir(workdir)
        .env("PATH", path)
        .env("PATHEXT", ".COM;.EXE;.BAT;.CMD");
    Ok(())
}

fn shim_error(plugin: &Path) -> GenerateError {
    schema(
        "ipc-plugin-shim",
        plugin.display().to_string(),
        "Windows batch plugins must be named protoc-gen-ts_proto.cmd or protoc-gen-ts_proto.bat",
        "pass the pinned npm installation's node_modules/.bin/protoc-gen-ts_proto.cmd path",
    )
}
