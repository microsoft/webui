// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FileIdentity {
    volume: u64,
    index: u64,
}

pub(super) struct OutputPathSet {
    normalized: HashSet<PathBuf>,
    identities: HashSet<FileIdentity>,
}

impl OutputPathSet {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            normalized: HashSet::with_capacity(capacity),
            identities: HashSet::with_capacity(capacity),
        }
    }

    pub fn insert(&mut self, path: &Path) -> std::io::Result<bool> {
        let resolved = resolved_absolute(path)?;
        let identity = file_identity(&resolved)?;
        if !self.normalized.insert(normalize_filesystem_case(resolved)) {
            return Ok(false);
        }
        if let Some(identity) = identity {
            if !self.identities.insert(identity) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn resolved_absolute(path: &Path) -> std::io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    resolve_output_path(&absolute, 0)
}

fn resolve_output_path(path: &Path, symlink_depth: usize) -> std::io::Result<PathBuf> {
    const MAX_SYMLINK_DEPTH: usize = 40;

    let mut resolved = PathBuf::with_capacity(path.as_os_str().len());
    let mut components = path.components();
    while let Some(component) = components.next() {
        match component {
            Component::Prefix(prefix) => resolved.push(prefix.as_os_str()),
            Component::RootDir => resolved.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(part) => {
                let candidate = resolved.join(part);
                match fs::symlink_metadata(&candidate) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        if symlink_depth >= MAX_SYMLINK_DEPTH {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidInput,
                                format!(
                                    "Too many symlinks while resolving output path {}",
                                    path.display()
                                ),
                            ));
                        }
                        let target = fs::read_link(&candidate)?;
                        let mut redirected = if target.is_absolute() {
                            target
                        } else {
                            resolved.join(target)
                        };
                        for remaining in components {
                            redirected.push(remaining.as_os_str());
                        }
                        return resolve_output_path(&redirected, symlink_depth + 1);
                    }
                    Ok(_) => resolved.push(part),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        resolved.push(part);
                    }
                    Err(error) => return Err(error),
                }
            }
        }
    }
    Ok(resolved)
}

#[cfg(any(windows, target_os = "macos"))]
fn normalize_filesystem_case(path: PathBuf) -> PathBuf {
    PathBuf::from(path.to_string_lossy().to_lowercase())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn normalize_filesystem_case(path: PathBuf) -> PathBuf {
    path
}

#[cfg(unix)]
fn file_identity(path: &Path) -> std::io::Result<Option<FileIdentity>> {
    use std::os::unix::fs::MetadataExt;

    match fs::metadata(path) {
        Ok(metadata) => Ok(Some(FileIdentity {
            volume: metadata.dev(),
            index: metadata.ino(),
        })),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
fn file_identity(path: &Path) -> std::io::Result<Option<FileIdentity>> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `file` owns a valid handle for the duration of the call and
    // `information` points to a writable output structure of the required type.
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle(), &raw mut information) };
    if succeeded == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(Some(FileIdentity {
        volume: u64::from(information.dwVolumeSerialNumber),
        index: (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow),
    }))
}

#[cfg(not(any(unix, windows)))]
fn file_identity(path: &Path) -> std::io::Result<Option<FileIdentity>> {
    match fs::metadata(path) {
        Ok(_) => Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
