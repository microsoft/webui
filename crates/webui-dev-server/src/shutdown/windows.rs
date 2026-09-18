// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(unsafe_code)]

use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::process::{Child, Command};
use windows_sys::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{WaitForSingleObject, CREATE_NEW_PROCESS_GROUP};

pub(super) struct Scope(OwnedHandle);

impl Scope {
    pub(super) fn new() -> io::Result<Self> {
        // SAFETY: Null security attributes create an unnamed, non-inherited job.
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: raw is a freshly created valid handle, transferred exactly once.
        let job = unsafe { OwnedHandle::from_raw_handle(raw) };
        // SAFETY: This Win32 C structure permits all-zero initialization.
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: The job handle and structure remain valid through the call.
        let result = unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                std::ptr::addr_of!(info).cast(),
                u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                    .map_err(io::Error::other)?,
            )
        };
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(job))
    }

    pub(super) fn configure(&self, command: &mut Command) {
        command.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }

    pub(super) fn attach(&mut self, child: &Child) -> io::Result<()> {
        // SAFETY: Both handles are owned and valid; the child's startup gate
        // prevents writers or descendants until after successful assignment.
        let result =
            unsafe { AssignProcessToJobObject(self.0.as_raw_handle(), child.as_raw_handle()) };
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn root_exited(&self, child: &Child) -> io::Result<bool> {
        // SAFETY: Child owns a valid process handle; a zero timeout never blocks.
        match unsafe { WaitForSingleObject(child.as_raw_handle(), 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            WAIT_FAILED => Err(io::Error::last_os_error()),
            _ => Err(io::Error::other("unexpected process wait result")),
        }
    }

    pub(super) fn descendants_after_exit(&self, child: &Child) -> io::Result<bool> {
        Ok(self.root_exited(child)? && self.active_count()? > 0)
    }

    pub(super) fn terminate(&self) -> io::Result<()> {
        // SAFETY: The job contains only the gated child and its descendants.
        if unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn empty(&self) -> io::Result<bool> {
        Ok(self.active_count()? == 0)
    }

    fn active_count(&self) -> io::Result<u32> {
        // SAFETY: This Win32 output structure permits all-zero initialization.
        let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
        // SAFETY: The output pointer covers the exact declared structure size.
        let result = unsafe {
            QueryInformationJobObject(
                self.0.as_raw_handle(),
                JobObjectBasicAccountingInformation,
                std::ptr::addr_of_mut!(info).cast(),
                u32::try_from(size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>())
                    .map_err(io::Error::other)?,
                std::ptr::null_mut(),
            )
        };
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(info.ActiveProcesses)
    }
}
