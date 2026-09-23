// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs::OpenOptions;
use std::io;
use std::mem::{align_of, offset_of, size_of};
use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::{
    ERROR_INVALID_FUNCTION, ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED,
};
use windows_sys::Win32::Storage::FileSystem::{
    FileRenameInfoEx, SetFileInformationByHandle, DELETE, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_RENAME_INFO, FILE_RENAME_INFO_0,
};

// FILE_RENAME_INFO.Flags values for FileRenameInfoEx.
const FILE_RENAME_FLAG_REPLACE_IF_EXISTS: u32 = 0x1;
const FILE_RENAME_FLAG_POSIX_SEMANTICS: u32 = 0x2;
const MAX_PATH_UNITS: usize = 32_767;

pub(super) fn replace(source: &Path, destination: &Path) -> io::Result<()> {
    let (buffer, size) = rename_info(destination)?;
    let file = OpenOptions::new()
        .access_mode(DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(source)?;
    // SAFETY: The live file handle has DELETE access. rename_info constructs a
    // correctly aligned FILE_RENAME_INFO followed by its bounded UTF-16 name,
    // with all bytes initialized and a size covering the complete allocation.
    let result = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileRenameInfoEx,
            buffer.as_ptr().cast(),
            size,
        )
    };
    if result == 0 {
        return Err(rename_error(io::Error::last_os_error()));
    }
    Ok(())
}

fn rename_info(destination: &Path) -> io::Result<(Vec<usize>, u32)> {
    let absolute = std::path::absolute(destination)?;
    let name: Vec<u16> = absolute.as_os_str().encode_wide().collect();
    if name.len() > MAX_PATH_UNITS || name.contains(&0) {
        return Err(invalid_destination());
    }
    let name_bytes = name.len() * size_of::<u16>();
    let bytes = (offset_of!(FILE_RENAME_INFO, FileName) + name_bytes + size_of::<u16>())
        .max(size_of::<FILE_RENAME_INFO>());
    let mut buffer = vec![0_usize; bytes.div_ceil(size_of::<usize>())];
    let size =
        u32::try_from(buffer.len() * size_of::<usize>()).map_err(|_| invalid_destination())?;
    let name_length = u32::try_from(name_bytes).map_err(|_| invalid_destination())?;
    const { assert!(align_of::<usize>() >= align_of::<FILE_RENAME_INFO>()) };
    let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    // SAFETY: The zeroed word buffer has FILE_RENAME_INFO alignment and room
    // for every fixed field, the entire name and a trailing NUL. Raw field
    // pointers avoid forming a reference limited to the one-element name array.
    // The buffer is not reallocated while these pointers are in use.
    unsafe {
        ptr::addr_of_mut!((*info).Anonymous).write(FILE_RENAME_INFO_0 {
            Flags: FILE_RENAME_FLAG_REPLACE_IF_EXISTS | FILE_RENAME_FLAG_POSIX_SEMANTICS,
        });
        ptr::addr_of_mut!((*info).RootDirectory).write(ptr::null_mut());
        ptr::addr_of_mut!((*info).FileNameLength).write(name_length);
        ptr::copy_nonoverlapping(
            name.as_ptr(),
            ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
            name.len(),
        );
    }
    Ok((buffer, size))
}

#[cold]
#[inline(never)]
fn invalid_destination() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "invalid window-state destination; help: use a path without NUL characters and at most 32767 UTF-16 code units",
    )
}

#[cold]
#[inline(never)]
fn rename_error(source: io::Error) -> io::Error {
    match source
        .raw_os_error()
        .and_then(|code| u32::try_from(code).ok())
    {
        Some(ERROR_INVALID_FUNCTION | ERROR_INVALID_PARAMETER | ERROR_NOT_SUPPORTED) => {
            io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "atomic window-state replacement is unavailable ({source}); help: use a local NTFS state directory on Windows 10 version 1607 or later, or disable remember_state"
                ),
            )
        }
        _ => source,
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    #[test]
    fn rename_descriptor_preserves_utf16_names_and_atomic_flags() {
        let path = Path::new("state-\u{65e5}\u{1f4be}.json");
        let expected: Vec<u16> = std::path::absolute(path)
            .unwrap()
            .as_os_str()
            .encode_wide()
            .collect();
        let (buffer, size) = rename_info(path).unwrap();
        assert_eq!(
            usize::try_from(size).unwrap(),
            buffer.len() * size_of::<usize>()
        );
        // SAFETY: rename_info initializes a properly aligned descriptor and its
        // complete name. The buffer remains live during these immutable reads.
        unsafe {
            let info = buffer.as_ptr().cast::<FILE_RENAME_INFO>();
            assert_eq!(
                (*info).Anonymous.Flags,
                FILE_RENAME_FLAG_REPLACE_IF_EXISTS | FILE_RENAME_FLAG_POSIX_SEMANTICS
            );
            assert!((*info).RootDirectory.is_null());
            assert_eq!(
                usize::try_from((*info).FileNameLength).unwrap(),
                expected.len() * 2
            );
            let name = std::slice::from_raw_parts(
                ptr::addr_of!((*info).FileName).cast::<u16>(),
                expected.len() + 1,
            );
            assert_eq!(&name[..expected.len()], expected.as_slice());
            assert_eq!(name[expected.len()], 0);
        }
    }

    #[test]
    fn rename_descriptor_rejects_embedded_nul_and_excessive_length() {
        let nul = OsString::from_wide(&[b's'.into(), 0, b'x'.into()]);
        let long = Path::new(r"\\?\C:\").join("x".repeat(MAX_PATH_UNITS));
        for path in [Path::new(&nul), long.as_path()] {
            assert_eq!(
                rename_info(path).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
        let relative = Path::new("s").join("x".repeat(MAX_PATH_UNITS));
        assert!(rename_info(&relative).is_err());
    }

    #[test]
    fn read_only_target_is_not_replaced_and_temporary_file_is_cleaned() {
        use crate::{WindowState, WindowStateStore};

        let dir = tempfile::tempdir().unwrap();
        let store = WindowStateStore::new(dir.path().join("state.json"));
        let state = WindowState {
            x: 10,
            y: 20,
            width: 800,
            height: 600,
            maximized: false,
        };
        store.save(&state).unwrap();
        let original = std::fs::read(store.path()).unwrap();
        let permissions = std::fs::metadata(store.path()).unwrap().permissions();
        let mut read_only = permissions.clone();
        read_only.set_readonly(true);
        std::fs::set_permissions(store.path(), read_only).unwrap();
        let result = store.save(&WindowState {
            width: 1200,
            ..state
        });
        std::fs::set_permissions(store.path(), permissions).unwrap();
        assert!(result.is_err());
        assert_eq!(std::fs::read(store.path()).unwrap(), original);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn unsupported_filesystems_are_reported_without_hiding_permission_failures() {
        for code in [
            ERROR_INVALID_FUNCTION,
            ERROR_INVALID_PARAMETER,
            ERROR_NOT_SUPPORTED,
        ] {
            let source = io::Error::from_raw_os_error(i32::try_from(code).unwrap());
            assert_eq!(rename_error(source).kind(), io::ErrorKind::Unsupported);
        }
        let denied = io::Error::from_raw_os_error(5);
        assert_eq!(rename_error(denied).raw_os_error(), Some(5));
    }
}
