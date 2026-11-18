// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(unsafe_code)]

use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::process::{Child, Command};
use std::time::Instant;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP;

pub(super) fn wait_for_exit(child: &mut Child) -> io::Result<()> {
    use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
    use windows_sys::Win32::System::Threading::{WaitForSingleObject, INFINITE};
    // SAFETY: The waiter exclusively owns Child and its valid process handle.
    if unsafe { WaitForSingleObject(child.as_raw_handle(), INFINITE) } == WAIT_OBJECT_0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub(super) fn interrupt_current_group() -> io::Result<()> {
    use windows_sys::Win32::System::Console::{GenerateConsoleCtrlEvent, CTRL_BREAK_EVENT};
    // SAFETY: The gated worker was created in its own console process group;
    // the event is addressed only to its own process id and descendants.
    if unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, std::process::id()) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

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

    pub(super) fn terminate(&self, _deadline: Instant) -> io::Result<()> {
        // SAFETY: The job contains only the gated child and its descendants.
        if unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
