// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::ffi::{c_void, OsStr};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use super::NativeServiceError;

#[link(name = "shell32")]
unsafe extern "system" {
    fn ShellExecuteW(
        window: *mut c_void,
        verb: *const u16,
        target: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
    ) -> isize;
}

pub(super) fn open_url(url: &str) -> Result<(), NativeServiceError> {
    open(OsStr::new(url))
}

pub(super) fn open_document(path: &Path) -> Result<(), NativeServiceError> {
    open(path.as_os_str())
}

fn open(target: &OsStr) -> Result<(), NativeServiceError> {
    let mut wide: Vec<u16> = target.encode_wide().collect();
    if wide.contains(&0) {
        return Err(NativeServiceError::InvalidDocument);
    }
    wide.push(0);
    let verb: Vec<u16> = OsStr::new("open").encode_wide().chain(Some(0)).collect();
    // SAFETY: null-terminated owned buffers remain alive during ShellExecuteW;
    // no shell command string or parameters are passed.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            wide.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        )
    };
    if result > 32 {
        Ok(())
    } else {
        Err(NativeServiceError::Os(format!(
            "ShellExecuteW returned {result}"
        )))
    }
}
