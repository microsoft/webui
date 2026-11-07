// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::windows::{check, Handle};
use std::mem::size_of;
use windows_sys::Win32::Foundation::{
    DuplicateHandle, DUPLICATE_SAME_ACCESS, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Threading::{
    DeleteProcThreadAttributeList, GetCurrentProcess, InitializeProcThreadAttributeList,
    UpdateProcThreadAttribute, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, STARTF_USESTDHANDLES,
    STARTUPINFOEXW,
};

pub(super) struct Stdio {
    // Attribute-list storage must remain aligned and live through CreateProcessW.
    _storage: Vec<usize>,
    _handles: Vec<Handle>,
    inherited: Vec<windows_sys::Win32::Foundation::HANDLE>,
    pub(super) startup: STARTUPINFOEXW,
}

#[allow(unsafe_code)]
impl Stdio {
    pub(super) fn new() -> Result<Self, String> {
        let mut handles = Vec::with_capacity(3);
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb =
            u32::try_from(size_of::<STARTUPINFOEXW>()).map_err(|error| error.to_string())?;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        for (id, destination) in [
            (STD_INPUT_HANDLE, &mut startup.StartupInfo.hStdInput),
            (STD_OUTPUT_HANDLE, &mut startup.StartupInfo.hStdOutput),
            (STD_ERROR_HANDLE, &mut startup.StartupInfo.hStdError),
        ] {
            // SAFETY: Reads the current process's standard handle table.
            let source = unsafe { GetStdHandle(id) };
            if source == INVALID_HANDLE_VALUE {
                return Err(super::windows::last_error("read standard handle"));
            }
            if source.is_null() {
                continue;
            }
            let mut duplicate = std::ptr::null_mut();
            // SAFETY: Duplicate only our own live handles, with identical access.
            // Never mutate inheritability of the parent's original handles.
            unsafe {
                let process = GetCurrentProcess();
                check(
                    DuplicateHandle(
                        process,
                        source,
                        process,
                        &mut duplicate,
                        0,
                        1,
                        DUPLICATE_SAME_ACCESS,
                    ),
                    "duplicate standard handle",
                )?;
            }
            *destination = duplicate;
            handles.push(Handle(duplicate));
        }
        let mut bytes = 0;
        // SAFETY: The first call queries the required allocation size.
        unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut bytes) };
        if bytes == 0 {
            return Err(super::windows::last_error("size process attribute list"));
        }
        let mut storage = vec![0_usize; bytes.div_ceil(size_of::<usize>())];
        let attributes = storage.as_mut_ptr().cast();
        // SAFETY: Aligned storage has the size requested by Windows.
        unsafe {
            check(
                InitializeProcThreadAttributeList(attributes, 1, 0, &mut bytes),
                "initialize process attribute list",
            )?;
        }
        startup.lpAttributeList = attributes;
        let result = Self {
            _storage: storage,
            inherited: handles.iter().map(|handle| handle.0).collect(),
            _handles: handles,
            startup,
        };
        if !result._handles.is_empty() {
            // SAFETY: The handle array stays live until attribute-list deletion.
            // Only our inheritable stdio duplicates may cross the process boundary.
            unsafe {
                check(
                    UpdateProcThreadAttribute(
                        attributes,
                        0,
                        PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                        result.inherited.as_ptr().cast(),
                        result.inherited.len()
                            * size_of::<windows_sys::Win32::Foundation::HANDLE>(),
                        std::ptr::null_mut(),
                        std::ptr::null(),
                    ),
                    "restrict inherited process handles",
                )?;
            }
        }
        Ok(result)
    }

    pub(super) fn inherits_handles(&self) -> bool {
        !self._handles.is_empty()
    }
}

#[allow(unsafe_code)]
impl Drop for Stdio {
    fn drop(&mut self) {
        // SAFETY: Initialization succeeded, and storage remains owned until after this call.
        unsafe { DeleteProcThreadAttributeList(self.startup.lpAttributeList) };
    }
}
