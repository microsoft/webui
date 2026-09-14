// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(
    unsafe_code,
    reason = "test-local Windows console and owned-job bindings"
)]

use std::ffi::c_void;
use std::mem::size_of;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr;

use anyhow::{ensure, Context, Result};
use windows_sys::Win32::Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
};

// These APIs are only needed by this test. Keep their ABI definitions local
// instead of expanding the runtime's windows-sys feature/dependency surface.
#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetConsoleCtrlHandler(
        handler: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
    fn GenerateConsoleCtrlEvent(event: u32, group: u32) -> i32;
    fn GetConsoleProcessList(processes: *mut u32, count: u32) -> u32;
    fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> HANDLE;
    fn SetInformationJobObject(job: HANDLE, class: i32, data: *const c_void, size: u32) -> i32;
    fn AssignProcessToJobObject(job: HANDLE, process: HANDLE) -> i32;
    fn QueryInformationJobObject(
        job: HANDLE,
        class: i32,
        data: *mut c_void,
        size: u32,
        returned: *mut u32,
    ) -> i32;
}

#[repr(C)]
#[derive(Default)]
struct BasicLimits {
    process_time: i64,
    job_time: i64,
    flags: u32,
    minimum_working_set: usize,
    maximum_working_set: usize,
    active_processes: u32,
    affinity: usize,
    priority: u32,
    scheduling: u32,
}

#[repr(C)]
#[derive(Default)]
struct ExtendedLimits {
    basic: BasicLimits,
    io_counters: [u64; 6],
    process_memory: usize,
    job_memory: usize,
    peak_process_memory: usize,
    peak_job_memory: usize,
}

#[repr(C)]
#[derive(Default)]
struct ProcessList {
    assigned: u32,
    count: u32,
    ids: [usize; 32],
}

pub(super) fn clear_inherited_ignore() -> Result<()> {
    // SAFETY: A null handler changes only this process's inheritable ignore flag.
    // Clear it before EVERY spawn, including after a preceding broadcast.
    check(unsafe { SetConsoleCtrlHandler(None, 0) }).context("clear inherited Ctrl+C ignore flag")
}

pub(super) fn broadcast(cli: u32, outer: u32) -> Result<()> {
    let mut processes = [0_u32; 32];
    // SAFETY: The writable buffer has exactly the advertised capacity.
    let count = unsafe { GetConsoleProcessList(processes.as_mut_ptr(), 32) } as usize;
    ensure!(
        count > 0 && count <= processes.len(),
        "invalid isolated console process list"
    );
    let attached = &processes[..count];
    ensure!(
        attached.contains(&std::process::id())
            && attached.contains(&cli)
            && !attached.contains(&outer),
        "refusing to signal anything other than the child-owned console: {attached:?}"
    );
    println!("REAL_CTRL_C_EVENT console processes: {attached:?}; CLI: {cli}");
    // SAFETY: Only this controller ignores the event, after its CLI was spawned.
    check(unsafe { SetConsoleCtrlHandler(None, 1) })?;
    // SAFETY: This helper is re-executed with CREATE_NEW_CONSOLE. The checks
    // above exclude the outer test/user console. Group 0 targets this console
    // only; CTRL_C_EVENT is 0 (not a synthetic Node signal or CTRL_BREAK_EVENT).
    check(unsafe { GenerateConsoleCtrlEvent(0, 0) }).context("broadcast real CTRL_C_EVENT")
}

pub(super) struct Job(OwnedHandle);

impl Job {
    pub(super) fn new() -> Result<Self> {
        // SAFETY: Null security/name select an unnamed, non-inheritable job.
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        ensure!(
            !handle.is_null(),
            "create console test job: {}",
            std::io::Error::last_os_error()
        );
        // SAFETY: The non-null, newly created handle is exclusively owned here.
        let job = Self(unsafe { OwnedHandle::from_raw_handle(handle) });
        let mut limits = ExtendedLimits::default();
        // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        limits.basic.flags = 0x2000;
        // SAFETY: Class 9 expects JOB_OBJECT_EXTENDED_LIMIT_INFORMATION with
        // exactly this repr(C) layout; all reserved fields are zero initialized.
        check(unsafe {
            SetInformationJobObject(
                job.0.as_raw_handle(),
                9,
                ptr::from_ref(&limits).cast(),
                u32::try_from(size_of::<ExtendedLimits>())?,
            )
        })?;
        Ok(job)
    }

    pub(super) fn assign(&self, process: &impl AsRawHandle) -> Result<()> {
        // SAFETY: Both borrowed handles remain live throughout this call.
        check(unsafe { AssignProcessToJobObject(self.0.as_raw_handle(), process.as_raw_handle()) })
            .context("contain owned console child before releasing its startup barrier")
    }

    pub(super) fn ids(&self) -> Result<Vec<u32>> {
        let mut list = ProcessList::default();
        // SAFETY: Class 3 writes a JOB_OBJECT_BASIC_PROCESS_ID_LIST into this
        // aligned repr(C) buffer, whose actual byte capacity is supplied.
        check(unsafe {
            QueryInformationJobObject(
                self.0.as_raw_handle(),
                3,
                ptr::from_mut(&mut list).cast(),
                u32::try_from(size_of::<ProcessList>())?,
                ptr::null_mut(),
            )
        })?;
        let count = usize::try_from(list.count)?;
        ensure!(
            count <= list.ids.len(),
            "console test process capacity exceeded"
        );
        list.ids[..count]
            .iter()
            .map(|id| u32::try_from(*id).map_err(Into::into))
            .collect()
    }

    pub(super) fn processes(&self) -> Result<Vec<Process>> {
        self.ids()?.into_iter().map(Process::open).collect()
    }
}

pub(super) struct Process {
    pub(super) id: u32,
    pub(super) image: String,
    handle: OwnedHandle,
}

impl Process {
    fn open(id: u32) -> Result<Self> {
        // SAFETY: The PID was obtained from our private job. The non-inheritable
        // handle pins process identity, so later checks cannot mistake PID reuse.
        let raw = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                id,
            )
        };
        ensure!(
            !raw.is_null(),
            "open owned process {id}: {}",
            std::io::Error::last_os_error()
        );
        // SAFETY: This successfully opened handle has a single RAII owner.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut path = [0_u16; 1024];
        let mut length = u32::try_from(path.len())?;
        // SAFETY: The output buffer is writable for its declared UTF-16 capacity.
        check(unsafe {
            QueryFullProcessImageNameW(handle.as_raw_handle(), 0, path.as_mut_ptr(), &mut length)
        })?;
        Ok(Self {
            id,
            image: String::from_utf16(&path[..usize::try_from(length)?])?,
            handle,
        })
    }

    pub(super) fn exited(&self) -> Result<bool> {
        self.wait(0)
    }

    pub(super) fn await_exit(&self) -> Result<bool> {
        self.wait(3000)
    }

    fn wait(&self, milliseconds: u32) -> Result<bool> {
        // SAFETY: This owned process handle stays live throughout the bounded wait.
        let result = unsafe { WaitForSingleObject(self.handle.as_raw_handle(), milliseconds) };
        ensure!(
            result == WAIT_OBJECT_0 || result == WAIT_TIMEOUT,
            "wait on owned process failed"
        );
        Ok(result == WAIT_OBJECT_0)
    }
}

fn check(result: i32) -> Result<()> {
    if result == 0 {
        Err(std::io::Error::last_os_error().into())
    } else {
        Ok(())
    }
}
