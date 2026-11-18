// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{invalid, SdkError};

#[derive(Serialize)]
pub(super) struct Sdk {
    pub(super) directory: PathBuf,
    pub(super) version: String,
    pub(super) target: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    schema_version: u32,
    version: String,
    target: String,
    os: String,
    arch: String,
    libc: Option<String>,
    files: BTreeMap<String, String>,
}

struct Platform {
    os: &'static str,
    arch: &'static str,
    target: &'static str,
    library: &'static str,
}

fn platform() -> Result<Platform, SdkError> {
    let (os, target_os, library) = match std::env::consts::OS {
        "macos" => ("darwin", "apple-darwin", "libwebui_ffi.dylib"),
        "linux" if cfg!(target_env = "gnu") => ("linux", "unknown-linux-gnu", "libwebui_ffi.so"),
        "windows" if cfg!(target_env = "msvc") => ("win32", "pc-windows-msvc", "webui_ffi.dll"),
        os => return Err(SdkError::Unsupported(os.to_owned())),
    };
    let (arch, target) = match (std::env::consts::ARCH, target_os) {
        ("aarch64", "apple-darwin") => ("arm64", "aarch64-apple-darwin"),
        ("x86_64", "apple-darwin") => ("x64", "x86_64-apple-darwin"),
        ("aarch64", "unknown-linux-gnu") => ("arm64", "aarch64-unknown-linux-gnu"),
        ("x86_64", "unknown-linux-gnu") => ("x64", "x86_64-unknown-linux-gnu"),
        ("aarch64", "pc-windows-msvc") => ("arm64", "aarch64-pc-windows-msvc"),
        ("x86_64", "pc-windows-msvc") => ("x64", "x86_64-pc-windows-msvc"),
        (arch, _) => return Err(SdkError::Unsupported(arch.to_owned())),
    };
    Ok(Platform {
        os,
        arch,
        target,
        library,
    })
}

pub(super) fn resolve(project: &Path, explicit: Option<&OsStr>) -> Result<Sdk, SdkError> {
    let platform = platform()?;
    let directory = match explicit {
        Some(value) if value.is_empty() => {
            return Err(invalid(project, "WEBUI_SDK_DIR must not be empty"));
        }
        Some(value) => PathBuf::from(value),
        None => project.join(".webui/sdk"),
    };
    let directory = fs::canonicalize(&directory).map_err(|source| SdkError::Read {
        path: directory,
        source,
    })?;
    let metadata_path = checked_file(&directory, "sdk.json")?;
    let file = open(&metadata_path)?;
    let mut bytes = Vec::with_capacity(4096);
    file.take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|source| SdkError::Read {
            path: metadata_path.clone(),
            source,
        })?;
    if bytes.len() > 65_536 {
        return Err(invalid(&metadata_path, "SDK metadata exceeds 64 KiB"));
    }
    let metadata: Metadata = serde_json::from_slice(&bytes)
        .map_err(|error| invalid(&metadata_path, error.to_string()))?;
    validate_identity(&directory, &metadata, &platform)?;
    validate_files(&directory, &metadata.files, &platform)?;
    Ok(Sdk {
        directory,
        version: metadata.version,
        target: metadata.target,
    })
}

fn validate_identity(
    root: &Path,
    metadata: &Metadata,
    platform: &Platform,
) -> Result<(), SdkError> {
    if metadata.schema_version != 1 || metadata.version != env!("CARGO_PKG_VERSION") {
        return Err(invalid(
            root,
            format!(
                "expected schema 1 and version {}, received schema {} and version {}",
                env!("CARGO_PKG_VERSION"),
                metadata.schema_version,
                metadata.version
            ),
        ));
    }
    let libc = if platform.os == "linux" {
        Some("gnu")
    } else {
        None
    };
    if metadata.target != platform.target
        || metadata.os != platform.os
        || metadata.arch != platform.arch
        || metadata.libc.as_deref() != libc
    {
        return Err(invalid(
            root,
            format!("expected platform {}", platform.target),
        ));
    }
    Ok(())
}

fn validate_files(
    root: &Path,
    files: &BTreeMap<String, String>,
    platform: &Platform,
) -> Result<(), SdkError> {
    let library = format!("lib/{}", platform.library);
    let mut inventory = vec!["include/webui_ffi.h", library.as_str(), "LICENSE"];
    if platform.os == "win32" {
        inventory.push("lib/webui_ffi.dll.lib");
    }
    if files.len() != inventory.len() {
        return Err(invalid(root, "SDK metadata has an invalid file inventory"));
    }
    let mut buffer = vec![0_u8; 16 * 1024];
    for relative in inventory {
        let expected = files
            .get(relative)
            .ok_or_else(|| invalid(root, format!("missing hash for {relative}")))?;
        if expected.len() != 64
            || !expected
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid(root, format!("invalid SHA-256 for {relative}")));
        }
        let path = checked_file(root, relative)?;
        let mut file = open(&path)?;
        let mut hash = Sha256::new();
        loop {
            let length = file.read(&mut buffer).map_err(|source| SdkError::Read {
                path: path.clone(),
                source,
            })?;
            if length == 0 {
                break;
            }
            hash.update(&buffer[..length]);
        }
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let matches = hash
            .finalize()
            .iter()
            .zip(expected.as_bytes().as_chunks::<2>().0)
            .all(|(byte, pair)| {
                pair[0] == HEX[usize::from(byte >> 4)] && pair[1] == HEX[usize::from(byte & 15)]
            });
        if !matches {
            return Err(invalid(&path, "SDK integrity check failed"));
        }
    }
    Ok(())
}

fn checked_file(root: &Path, relative: &str) -> Result<PathBuf, SdkError> {
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path).map_err(|source| SdkError::Read {
        path: path.clone(),
        source,
    })?;
    if !metadata.file_type().is_file() {
        return Err(invalid(
            &path,
            "SDK inventory entries must be regular files, not symlinks",
        ));
    }
    let canonical = fs::canonicalize(&path).map_err(|source| SdkError::Read {
        path: path.clone(),
        source,
    })?;
    if !canonical.starts_with(root) {
        return Err(invalid(
            &path,
            "SDK inventory entry escapes the SDK directory",
        ));
    }
    Ok(path)
}

fn open(path: &Path) -> Result<File, SdkError> {
    File::open(path).map_err(|source| SdkError::Read {
        path: path.to_owned(),
        source,
    })
}
