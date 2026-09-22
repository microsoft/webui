// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Check the opened handle, not just the name checked before opening. This
//! closes the canonicalize/open symlink substitution gap without buffering files.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use crate::{DesktopError, Result};

pub(crate) fn open(root: &Path, path: &Path) -> Result<File> {
    let file = File::open(path).map_err(|source| DesktopError::Io {
        context: format!("opening desktop asset {}", path.display()),
        source,
    })?;
    validate(root, &file)?;
    Ok(file)
}

fn validate(root: &Path, file: &File) -> Result<()> {
    let opened = opened_path(file).map_err(|source| DesktopError::Io {
        context: "checking opened desktop asset confinement".into(),
        source,
    })?;
    if !opened.starts_with(root) {
        return Err(DesktopError::InvalidAssetPath {
            path: opened.display().to_string(),
        });
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn opened_path(file: &File) -> io::Result<PathBuf> {
    use std::os::fd::AsRawFd;
    // procfs resolves the actual handle even when an ancestor was substituted.
    std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn opened_path(file: &File) -> io::Result<PathBuf> {
    use std::ffi::OsString;
    use std::os::{fd::AsRawFd, unix::ffi::OsStringExt};
    let mut buffer = vec![0_u8; libc::PATH_MAX as usize];
    // SAFETY: F_GETPATH requires a live fd and a writable PATH_MAX buffer.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, buffer.as_mut_ptr()) } == -1 {
        return Err(io::Error::last_os_error());
    }
    let length = buffer.iter().position(|byte| *byte == 0).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "unterminated desktop asset path",
        )
    })?;
    buffer.truncate(length);
    Ok(PathBuf::from(OsString::from_vec(buffer)))
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn opened_path(file: &File) -> io::Result<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::{ffi::OsStringExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
    const CAPACITY: u32 = 32768;
    let mut buffer = vec![0_u16; CAPACITY as usize];
    // SAFETY: The handle stays live, and the buffer holds CAPACITY UTF-16 units.
    // Flags 0 request the normalized DOS path, matching std::fs::canonicalize.
    let length = unsafe {
        GetFinalPathNameByHandleW(file.as_raw_handle(), buffer.as_mut_ptr(), CAPACITY, 0)
    };
    if length == 0 {
        return Err(io::Error::last_os_error());
    }
    if length >= CAPACITY {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "desktop asset path too long",
        ));
    }
    Ok(PathBuf::from(OsString::from_wide(
        &buffer[..length as usize],
    )))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn opened_path(_file: &File) -> io::Result<PathBuf> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "desktop asset confinement requires macOS, Windows, or Linux",
    ))
}

#[cfg(all(test, unix))]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn checks_open_handle_after_parent_symlink_substitution() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join("nested/file"), b"inside").unwrap();
        std::fs::write(outside.path().join("file"), b"outside").unwrap();
        let canonical = root.join("nested/file").canonicalize().unwrap();
        std::fs::remove_dir_all(root.join("nested")).unwrap();
        symlink(outside.path(), root.join("nested")).unwrap();
        assert!(open(&root, &canonical).is_err());
    }
}
