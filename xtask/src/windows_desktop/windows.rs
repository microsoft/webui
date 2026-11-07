// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(unsafe_code)]

use super::{quote_argument, stdio::Stdio, Request};
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Foundation::{
    CloseHandle, HANDLE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::StationsAndDesktops::{
    CloseDesktop, CreateDesktopW, GetThreadDesktop, GetUserObjectInformationW, OpenInputDesktop,
    DESKTOP_CREATEWINDOW, DESKTOP_READOBJECTS, DESKTOP_WRITEOBJECTS, HDESK, UOI_NAME,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, GetCurrentThreadId, GetExitCodeProcess, ResumeThread, TerminateProcess,
    WaitForSingleObject, CREATE_SUSPENDED, EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION,
};

pub(super) fn execute(request: Request<'_>, isolated: bool) -> Result<u32, String> {
    let desktop = if isolated {
        Some(Desktop::new()?)
    } else {
        None
    };
    launch(request, desktop.as_ref())
}

pub(super) fn is_current() -> Result<bool, String> {
    // SAFETY: Reads this thread's desktop; no handle ownership is transferred.
    let current = desktop_name(unsafe { GetThreadDesktop(GetCurrentThreadId()) })?;
    if !current.starts_with("WebUI-Tests-") {
        return Ok(false);
    }
    // SAFETY: Opens the input desktop only for reading, never for switching.
    let input = unsafe { OpenInputDesktop(0, 0, DESKTOP_READOBJECTS) };
    if input.is_null() {
        return Err(last_error("read input desktop for test isolation"));
    }
    let name = desktop_name(input);
    // SAFETY: This function owns the OpenInputDesktop reference.
    check(unsafe { CloseDesktop(input) }, "close input desktop")?;
    Ok(current != name?)
}

fn desktop_name(handle: HDESK) -> Result<String, String> {
    let mut buffer = [0_u16; 256];
    // SAFETY: The handle is live and the buffer has the specified byte capacity.
    check(
        unsafe {
            GetUserObjectInformationW(
                handle,
                UOI_NAME,
                buffer.as_mut_ptr().cast(),
                u32::try_from(size_of_val(&buffer)).map_err(|error| error.to_string())?,
                std::ptr::null_mut(),
            )
        },
        "read test desktop name",
    )?;
    let length = buffer
        .iter()
        .position(|unit| *unit == 0)
        .ok_or("Windows returned an unterminated desktop name")?;
    String::from_utf16(&buffer[..length]).map_err(|error| error.to_string())
}

fn launch(request: Request<'_>, desktop: Option<&Desktop>) -> Result<u32, String> {
    let executable = which::which(&request.command[0])
        .map_err(|error| format!("cannot locate {}: {error}", request.command[0]))?;
    if !executable
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        return Err(
            "requires a native .exe; help: launch scripts through their interpreter".into(),
        );
    }
    let executable_wide: Vec<_> = executable
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut command = Vec::with_capacity(256);
    quote_argument(&mut command, executable.as_os_str().encode_wide());
    for arg in &request.command[1..] {
        command.push(u16::from(b' '));
        quote_argument(&mut command, arg.encode_utf16());
    }
    command.push(0);
    if command.len() > 32_767 {
        return Err("command exceeds the Windows 32767 UTF-16 code-unit limit".into());
    }
    let job = job()?;
    let mut stdio = Stdio::new()?;
    if let Some(desktop) = desktop {
        stdio.startup.StartupInfo.lpDesktop = desktop.name.as_ptr().cast_mut();
    }
    let mut information = PROCESS_INFORMATION::default();
    // SAFETY: All strings are terminated; stdio storage and the desktop outlive
    // creation. Suspend before assigning the job so no descendants can escape it.
    unsafe {
        check(
            CreateProcessW(
                executable_wide.as_ptr(),
                command.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                i32::from(stdio.inherits_handles()),
                CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT,
                std::ptr::null(),
                std::ptr::null(),
                &stdio.startup.StartupInfo,
                &mut information,
            ),
            "launch isolated test process",
        )?;
    }
    let mut child = Child {
        process: Handle(information.hProcess),
        finished: false,
    };
    let thread = Handle(information.hThread);
    // SAFETY: Both handles belong to the suspended child we just created.
    unsafe {
        check(
            AssignProcessToJobObject(job.0, child.process.0),
            "assign isolated test process to cleanup job",
        )?;
        if ResumeThread(thread.0) == u32::MAX {
            return Err(last_error("resume isolated test process"));
        }
    }
    drop(thread);
    drop(stdio);
    // SAFETY: The child handle stays owned throughout this bounded kernel wait.
    match unsafe { WaitForSingleObject(child.process.0, request.timeout_ms) } {
        WAIT_OBJECT_0 => {}
        WAIT_TIMEOUT => {
            return Err("test command timed out; its process tree was terminated".into())
        }
        WAIT_FAILED => return Err(last_error("wait for isolated test process")),
        status => return Err(format!("unexpected Windows wait result: {status}")),
    }
    child.finished = true;
    let mut code = 0;
    // SAFETY: The terminated child process handle is still live.
    unsafe {
        check(
            GetExitCodeProcess(child.process.0, &mut code),
            "read test exit code",
        )?
    };
    // Closing the job kills any remaining browser/fixture descendants before
    // releasing our desktop handle. There is no polling or per-test desktop.
    drop(job);
    Ok(code)
}

struct Desktop {
    handle: HDESK,
    name: Vec<u16>,
}

impl Desktop {
    fn new() -> Result<Self, String> {
        static NEXT_DESKTOP: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("cannot name test desktop: {error}"))?
            .as_nanos();
        let sequence = NEXT_DESKTOP.fetch_add(1, Ordering::Relaxed);
        let name: Vec<_> = format!("WebUI-Tests-{}-{nonce}-{sequence}", std::process::id())
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // SAFETY: A terminated unique name, no device/mode, default security,
        // and a non-inheritable handle. Never request or call SwitchDesktop.
        let handle = unsafe {
            CreateDesktopW(
                name.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                DESKTOP_CREATEWINDOW | DESKTOP_READOBJECTS | DESKTOP_WRITEOBJECTS,
                std::ptr::null(),
            )
        };
        if handle.is_null() {
            return Err(last_error(
                "create test desktop; help: run as an interactive Windows user",
            ));
        }
        Ok(Self { handle, name })
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        // SAFETY: This guard owns a CreateDesktopW reference, not a thread desktop.
        if unsafe { CloseDesktop(self.handle) } == 0 {
            eprintln!("isolated desktop: {}", last_error("close test desktop"));
        }
    }
}

pub(super) struct Handle(pub(super) HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: This guard owns exactly one live kernel handle.
        if unsafe { CloseHandle(self.0) } == 0 {
            eprintln!(
                "isolated desktop: {}",
                last_error("close process/job handle")
            );
        }
    }
}

struct Child {
    process: Handle,
    finished: bool,
}

impl Drop for Child {
    fn drop(&mut self) {
        if !self.finished {
            // SAFETY: The owned process may still be suspended if job assignment failed.
            if unsafe { TerminateProcess(self.process.0, 1) } == 0 {
                eprintln!(
                    "isolated desktop: {}",
                    last_error("terminate unfinished child")
                );
            }
        }
    }
}

fn job() -> Result<Handle, String> {
    // SAFETY: Null security/name create an unnamed, non-inheritable job.
    let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if raw.is_null() {
        return Err(last_error("create process cleanup job"));
    }
    let handle = Handle(raw);
    let mut information = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: The live job receives a correctly sized and initialized structure.
    unsafe {
        check(
            SetInformationJobObject(
                handle.0,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&information).cast(),
                u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                    .map_err(|error| error.to_string())?,
            ),
            "configure process-tree cleanup",
        )?;
    }
    Ok(handle)
}

pub(super) fn check(success: i32, operation: &str) -> Result<(), String> {
    if success == 0 {
        Err(last_error(operation))
    } else {
        Ok(())
    }
}

#[cold]
#[inline(never)]
pub(super) fn last_error(operation: &str) -> String {
    format!("{operation}: {}", std::io::Error::last_os_error())
}

#[cfg(test)]
mod tests;
