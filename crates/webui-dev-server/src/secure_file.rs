// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Filesystem-root confinement for static file serving.

use std::fs::File;
use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct SecureRoot {
    path: PathBuf,
    #[cfg(unix)]
    directory: Arc<File>,
}

impl SecureRoot {
    pub(crate) fn new(path: PathBuf) -> std::io::Result<Self> {
        let path = std::fs::canonicalize(path)?;
        #[cfg(unix)]
        let directory = Arc::new(File::open(&path)?);
        Ok(Self {
            path,
            #[cfg(unix)]
            directory,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn join(&self, path: &Path) -> PathBuf {
        self.path.join(path)
    }

    #[cfg(unix)]
    pub(crate) fn open(&self, path: PathBuf) -> std::io::Result<OpenedNode> {
        use std::os::fd::AsRawFd;
        use std::path::Component;

        let relative = path.strip_prefix(&self.path).map_err(|_| {
            Error::new(ErrorKind::PermissionDenied, "path escapes the serving root")
        })?;
        let mut components = relative.components().peekable();
        let mut parent = None::<File>;
        while let Some(component) = components.next() {
            let Component::Normal(name) = component else {
                return Err(Error::new(
                    ErrorKind::PermissionDenied,
                    "path contains an unsafe component",
                ));
            };
            let parent_descriptor = parent
                .as_ref()
                .map_or_else(|| self.directory.as_raw_fd(), AsRawFd::as_raw_fd);
            if components.peek().is_none() {
                let metadata = metadata_at(parent_descriptor, name)?;
                if metadata.file_type == libc::S_IFDIR {
                    return Ok(OpenedNode::Directory);
                }
                if metadata.file_type != libc::S_IFREG {
                    return Err(Error::new(
                        ErrorKind::InvalidInput,
                        "cannot serve a non-regular file",
                    ));
                }
                let file = open_at(parent_descriptor, name, false)?;
                return Ok(OpenedNode::File {
                    path,
                    file,
                    length: metadata.length,
                });
            }
            let child = open_at(parent_descriptor, name, true)?;
            parent = Some(child);
        }
        Err(Error::new(
            ErrorKind::InvalidInput,
            "path does not identify a file",
        ))
    }

    #[cfg(not(unix))]
    pub(crate) fn open(&self, path: PathBuf) -> std::io::Result<OpenedNode> {
        let path = std::fs::canonicalize(path)?;
        if !path.starts_with(&self.path) {
            return Err(Error::new(
                ErrorKind::PermissionDenied,
                "path escapes the serving root",
            ));
        }
        let file = File::open(&path)?;
        let metadata = file.metadata()?;
        if metadata.is_dir() {
            return Ok(OpenedNode::Directory);
        }
        if !metadata.is_file() {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "cannot serve a non-regular file",
            ));
        }
        Ok(OpenedNode::File {
            path,
            file,
            length: metadata.len(),
        })
    }
}

pub(crate) enum OpenedNode {
    File {
        path: PathBuf,
        file: File,
        length: u64,
    },
    Directory,
}

#[cfg(unix)]
struct EntryMetadata {
    file_type: libc::mode_t,
    length: u64,
}

#[cfg(unix)]
fn open_at(
    parent: std::os::fd::RawFd,
    name: &std::ffi::OsStr,
    directory: bool,
) -> std::io::Result<File> {
    use std::ffi::CString;
    use std::os::fd::FromRawFd;
    use std::os::unix::ffi::OsStrExt;

    let name = CString::new(name.as_bytes())
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "path component contains NUL"))?;
    let mut flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK;
    if directory {
        flags |= libc::O_DIRECTORY;
    }
    // SAFETY: `parent` is a live directory descriptor, `name` is a
    // NUL-terminated single component, and no pointer outlives this call.
    let descriptor = unsafe { libc::openat(parent, name.as_ptr(), flags) };
    if descriptor < 0 {
        return Err(Error::last_os_error());
    }
    // SAFETY: `openat` returned a new owned descriptor that is transferred
    // exactly once into `File`.
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

#[cfg(unix)]
fn metadata_at(
    parent: std::os::fd::RawFd,
    name: &std::ffi::OsStr,
) -> std::io::Result<EntryMetadata> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let name = CString::new(name.as_bytes())
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "path component contains NUL"))?;
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: `parent` is a live directory descriptor, `name` is a
    // NUL-terminated single component, and `metadata` points to enough
    // writable memory for `fstatat` to initialize.
    let result = unsafe {
        libc::fstatat(
            parent,
            name.as_ptr(),
            metadata.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result < 0 {
        return Err(Error::last_os_error());
    }
    // SAFETY: successful `fstatat` initialized the complete `stat` value.
    let metadata = unsafe { metadata.assume_init() };
    let length = u64::try_from(metadata.st_size)
        .map_err(|_| Error::new(ErrorKind::InvalidData, "file has a negative length"))?;
    Ok(EntryMetadata {
        file_type: metadata.st_mode & libc::S_IFMT,
        length,
    })
}
