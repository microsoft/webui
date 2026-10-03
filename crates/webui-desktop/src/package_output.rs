// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Package-only output handles; Unix writes remain relative to the new root fd.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use crate::error::{DesktopError, Result};

#[cfg(unix)]
#[allow(unsafe_code)]
mod unix {
    use super::*;
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::OpenOptionsExt;

    pub(super) struct OutputRoot {
        root: OwnedFd,
        path: PathBuf,
        created_dirs: Vec<PathBuf>,
    }

    fn c_name(name: &OsStr) -> Result<CString> {
        CString::new(name.as_bytes()).map_err(|_| DesktopError::InvalidAssetPath {
            path: name.to_string_lossy().into_owned(),
        })
    }

    fn open_dir(parent: &OwnedFd, name: &OsStr) -> Result<OwnedFd> {
        let name = c_name(name)?;
        // SAFETY: `name` is NUL-terminated; `parent` remains owned and open.
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io_error(
                "opening package directory",
                std::io::Error::last_os_error(),
            ));
        }
        // SAFETY: a successful `openat` transfers ownership of this fresh fd.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    impl OutputRoot {
        pub(super) fn create(parent: &Path, name: &OsStr) -> Result<Self> {
            fs::create_dir_all(parent)
                .map_err(|source| io_error("creating package parent", source))?;
            let parent_fd: OwnedFd = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
                .open(parent)
                .map_err(|source| {
                    io_error("opening package parent without following links", source)
                })?
                .into();
            let name_c = c_name(name)?;
            // SAFETY: parent fd is open; `name_c` is NUL-terminated; exclusive
            // mkdir creates only a new private directory in that parent.
            if unsafe { libc::mkdirat(parent_fd.as_raw_fd(), name_c.as_ptr(), 0o700) } != 0 {
                return Err(io_error(
                    "creating exclusive package root",
                    std::io::Error::last_os_error(),
                ));
            }
            let root = open_dir(&parent_fd, name)?;
            Ok(Self {
                root,
                path: parent.join(name),
                created_dirs: Vec::new(),
            })
        }

        fn directory(&mut self, relative: &Path) -> Result<OwnedFd> {
            let mut current = self
                .root
                .try_clone()
                .map_err(|source| io_error("cloning package root handle", source))?;
            let mut path = PathBuf::new();
            for component in relative.components() {
                let name = component.as_os_str();
                path.push(name);
                let name_c = c_name(name)?;
                // SAFETY: current is an owned directory fd; path components
                // were validated before packaging and contain no separators.
                if unsafe { libc::mkdirat(current.as_raw_fd(), name_c.as_ptr(), 0o700) } == 0 {
                    self.created_dirs.push(path.clone());
                } else {
                    let source = std::io::Error::last_os_error();
                    if source.kind() != std::io::ErrorKind::AlreadyExists {
                        return Err(io_error("creating package subdirectory", source));
                    }
                }
                current = open_dir(&current, name)?;
            }
            Ok(current)
        }

        pub(super) fn create_file(&mut self, relative: &Path) -> Result<File> {
            let parent = relative.parent().unwrap_or_else(|| Path::new(""));
            let dir = self.directory(parent)?;
            let name = relative
                .file_name()
                .ok_or_else(|| DesktopError::InvalidAssetPath {
                    path: relative.display().to_string(),
                })?;
            let name_c = c_name(name)?;
            // SAFETY: dir is an owned package directory fd; O_EXCL and
            // O_NOFOLLOW prevent replacing an existing file or symlink.
            let fd = unsafe {
                libc::openat(
                    dir.as_raw_fd(),
                    name_c.as_ptr(),
                    libc::O_RDWR
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC,
                    0o600,
                )
            };
            if fd < 0 {
                return Err(io_error(
                    "creating exclusive package file",
                    std::io::Error::last_os_error(),
                ));
            }
            // SAFETY: successful `openat` returns a fresh owned file descriptor.
            Ok(unsafe { File::from_raw_fd(fd) })
        }

        pub(super) fn finish(self) -> Result<()> {
            let opened = File::from(
                self.root
                    .try_clone()
                    .map_err(|source| io_error("checking package root", source))?,
            );
            let owned = opened
                .metadata()
                .map_err(|source| io_error("checking package root handle", source))?;
            let named = fs::symlink_metadata(&self.path)
                .map_err(|source| io_error("checking package root path", source))?;
            if !named.is_dir() || named.dev() != owned.dev() || named.ino() != owned.ino() {
                return Err(DesktopError::PackageValidation {
                    message: format!(
                        "package root {} was replaced during copying",
                        self.path.display()
                    ),
                    help:
                        "Choose a trusted output parent directory and retry with a new output path",
                });
            }
            for relative in &self.created_dirs {
                let parent = relative.parent().unwrap_or_else(|| Path::new(""));
                let mut current = self
                    .root
                    .try_clone()
                    .map_err(|source| io_error("cloning package root", source))?;
                for component in parent.components() {
                    current = open_dir(&current, component.as_os_str())?;
                }
                let dir = open_dir(
                    &current,
                    relative
                        .file_name()
                        .ok_or_else(|| DesktopError::InvalidAssetPath {
                            path: relative.display().to_string(),
                        })?,
                )?;
                // SAFETY: this fd names the directory created under our root.
                if unsafe { libc::fchmod(dir.as_raw_fd(), 0o755) } != 0 {
                    return Err(io_error(
                        "setting package directory permissions",
                        std::io::Error::last_os_error(),
                    ));
                }
            }
            // SAFETY: the root fd remains valid until `self` is dropped.
            if unsafe { libc::fchmod(self.root.as_raw_fd(), 0o755) } != 0 {
                return Err(io_error(
                    "setting package root permissions",
                    std::io::Error::last_os_error(),
                ));
            }
            Ok(())
        }
    }
}

#[cfg(unix)]
use unix::OutputRoot;

#[cfg(windows)]
struct OutputRoot {
    path: PathBuf,
}

#[cfg(windows)]
impl OutputRoot {
    fn create(parent: &Path, name: &OsStr) -> Result<Self> {
        fs::create_dir_all(parent).map_err(|source| io_error("creating package parent", source))?;
        let path = parent.join(name);
        fs::create_dir(&path)
            .map_err(|source| io_error("creating exclusive package root", source))?;
        Ok(Self { path })
    }

    fn create_file(&mut self, relative: &Path) -> Result<File> {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|source| io_error("creating package subdirectory", source))?;
        }
        OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|source| io_error("creating exclusive package file", source))
    }

    fn finish(self) -> Result<()> {
        Ok(())
    }
}

/// A newly created package root, owned through the last write.
pub(super) struct PackageOutput(OutputRoot);

impl PackageOutput {
    pub(super) fn create(parent: &Path, name: &OsStr) -> Result<Self> {
        Ok(Self(OutputRoot::create(parent, name)?))
    }

    pub(super) fn create_file(&mut self, relative: &Path) -> Result<File> {
        self.0.create_file(relative)
    }

    pub(super) fn finish(self) -> Result<()> {
        self.0.finish()
    }
}

fn io_error(context: &str, source: std::io::Error) -> DesktopError {
    DesktopError::Io {
        context: context.to_string(),
        source,
    }
}
