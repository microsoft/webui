// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Cross-platform child-process lifecycle management.
//!
//! Provides spawning children in their own process group and graceful shutdown
//! via SIGTERM (Unix) or `CTRL_BREAK_EVENT` (Windows), with a timed fallback
//! to a forced kill.
//!
//! On Windows, spawned children are assigned to a Job Object so the entire
//! process tree is terminated when the handle is dropped  even when the direct
//! child is a `cmd.exe /c` wrapper.

use std::path::Path;
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::util;

/// Maximum time to wait for a child to exit after a graceful stop signal before
/// escalating to a forced kill.
const GRACEFUL_TIMEOUT: Duration = Duration::from_secs(3);

/// A port that must remain free before starting a managed process group.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReservedPort<'a> {
    label: &'a str,
    port: u16,
}

impl<'a> ReservedPort<'a> {
    #[must_use]
    pub const fn new(label: &'a str, port: u16) -> Self {
        Self { label, port }
    }
}

// ---------------------------------------------------------------------------
// ManagedChild
// ---------------------------------------------------------------------------

/// A child process with platform-specific process-tree tracking.
///
/// On Windows this wraps the child in a Job Object so that `cmd.exe /c`
/// wrapper processes and their descendants are cleaned up on drop.
pub struct ManagedChild {
    inner: Child,
    #[cfg(windows)]
    _job: sys::JobHandle,
    #[cfg(windows)]
    pending_exit: Option<std::process::ExitStatus>,
}

impl ManagedChild {
    /// Check if the child has exited without blocking.
    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        #[cfg(windows)]
        if self.pending_exit.is_some() {
            if self._job.has_active_processes() {
                return Ok(None);
            }
            return Ok(self.pending_exit.take());
        }

        let status = self.inner.try_wait()?;

        #[cfg(windows)]
        if let Some(status) = status {
            if self._job.has_active_processes() {
                self.pending_exit = Some(status);
                return Ok(None);
            }
            return Ok(Some(status));
        }

        Ok(status)
    }

    /// Block until the child exits and return its status.
    pub fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        #[cfg(windows)]
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            thread::sleep(Duration::from_millis(50));
        }

        #[cfg(not(windows))]
        self.inner.wait()
    }

    /// Force-kill the child process.
    pub fn kill(&mut self) -> std::io::Result<()> {
        #[cfg(windows)]
        {
            self._job.terminate_all();
            match self.inner.kill() {
                Ok(()) => Ok(()),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::InvalidInput | std::io::ErrorKind::NotFound
                    ) =>
                {
                    Ok(())
                }
                Err(error) => Err(error),
            }
        }

        #[cfg(not(windows))]
        self.inner.kill()
    }
}

// ---------------------------------------------------------------------------
// Spawning
// ---------------------------------------------------------------------------

/// Internal: configure and spawn a command in its own process group with
/// a Job Object on Windows.
fn spawn_managed(command: &mut Command) -> Result<ManagedChild, std::io::Error> {
    sys::configure_process_group(command);
    let child = command.spawn()?;

    #[cfg(windows)]
    let _job = sys::JobHandle::attach(&child);

    Ok(ManagedChild {
        inner: child,
        #[cfg(windows)]
        _job,
        #[cfg(windows)]
        pending_exit: None,
    })
}

/// Shared flag to suppress output during shutdown.
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

/// Spawn a labelled child process with prefixed output.
///
/// Each line of stdout and stderr is printed with a colored `[label]` prefix.
/// Reader threads handle the prefixing and terminate when the child's pipes close.
/// Output is suppressed once [`SHUTTING_DOWN`] is set (during Ctrl+C).
pub fn spawn_child_prefixed(
    label: &str,
    cmd: &str,
    args: &[&str],
    cwd: &Path,
    color: console::Color,
) -> Option<ManagedChild> {
    let mut command = util::build_command(cmd, args);
    // Use piped stdin (not null) — tools like esbuild --watch exit when stdin closes.
    // A piped stdin stays open until the parent drops it.
    command
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    match spawn_managed(&mut command) {
        Ok(mut managed) => {
            fn pipe_reader<R: std::io::Read + Send + 'static>(
                pipe: R,
                tag: String,
                color: console::Color,
            ) {
                thread::spawn(move || {
                    use std::io::BufRead;
                    for line in std::io::BufReader::new(pipe).lines() {
                        let Ok(line) = line else { break };
                        if SHUTTING_DOWN.load(Ordering::SeqCst) {
                            break;
                        }
                        eprintln!(
                            "  {} {}",
                            console::style(format!("[{tag}]")).fg(color).bold(),
                            line,
                        );
                    }
                });
            }
            if let Some(stdout) = managed.inner.stdout.take() {
                pipe_reader(stdout, label.to_string(), color);
            }
            if let Some(stderr) = managed.inner.stderr.take() {
                pipe_reader(stderr, label.to_string(), color);
            }
            Some(managed)
        }
        Err(e) => {
            eprintln!(
                "  {} [{}] failed to start: {}",
                console::style("✘").red().bold(),
                label,
                e
            );
            None
        }
    }
}

/// Spawn a labelled child process with suppressed I/O.
///
/// All output goes to null — useful for servers whose shutdown noise
/// (e.g. pnpm printing `STATUS_CONTROL_C_EXIT`) should not appear.
pub fn spawn_child_quiet(
    label: &str,
    cmd: &str,
    args: &[&str],
    cwd: &Path,
) -> Option<ManagedChild> {
    let mut command = util::build_command(cmd, args);
    command
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    match spawn_managed(&mut command) {
        Ok(managed) => Some(managed),
        Err(e) => {
            eprintln!(
                "  {} [{}] failed to start: {}",
                console::style("✘").red().bold(),
                label,
                e
            );
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Port pre-check: shared reserved-port validation
// ---------------------------------------------------------------------------

/// Return an error when one or more required ports are already occupied.
pub fn ensure_reserved_ports_available(
    target: &str,
    ports: &[ReservedPort<'_>],
) -> Result<(), String> {
    let occupied: Vec<String> = ports
        .iter()
        .filter(|entry| !is_local_port_available(entry.port))
        .map(|entry| format!("{}={}", entry.label, entry.port))
        .collect();

    if occupied.is_empty() {
        return Ok(());
    }

    let noun = if occupied.len() == 1 {
        "port is"
    } else {
        "ports are"
    };
    Err(format!(
        "Cannot start {target}: required {noun} already in use ({})",
        occupied.join(", ")
    ))
}

fn is_local_port_available(port: u16) -> bool {
    std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .map(drop)
        .is_ok()
}

// ---------------------------------------------------------------------------
// Graceful shutdown
// ---------------------------------------------------------------------------
pub fn terminate_gracefully(managed: &mut ManagedChild) {
    sys::send_graceful_stop(&managed.inner);

    let deadline = Instant::now() + GRACEFUL_TIMEOUT;
    while Instant::now() < deadline {
        if matches!(managed.try_wait(), Ok(Some(_))) {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }

    let _ = managed.kill();
    let _ = managed.wait();
}

/// Run a Ctrl+C–aware poll loop for a group of named child processes.
///
/// * On Ctrl+C all children receive a graceful stop signal in parallel.
/// * If any child exits on its own the others are also stopped.
/// * Returns [`ExitCode::SUCCESS`] for user-initiated stops, or
///   [`ExitCode::FAILURE`] if a child crashed.
pub fn wait_for_group(children: &mut [(&str, ManagedChild)]) -> ExitCode {
    let ctrlc = Arc::new(AtomicBool::new(false));
    let flag = ctrlc.clone();
    if let Err(e) = ctrlc::set_handler(move || {
        flag.store(true, Ordering::SeqCst);
    }) {
        eprintln!("warning: failed to set Ctrl+C handler: {e}");
    }

    loop {
        if ctrlc.load(Ordering::SeqCst) {
            SHUTTING_DOWN.store(true, Ordering::SeqCst);
            shutdown_group(children);
            eprintln!("\n  {} Stopped gracefully", console::style("👋").green(),);
            return ExitCode::SUCCESS;
        }

        let any_done = children
            .iter_mut()
            .any(|(_, c)| matches!(c.try_wait(), Ok(Some(_))));

        if any_done {
            SHUTTING_DOWN.store(true, Ordering::SeqCst);

            // Terminate every child that hasn't exited yet.
            for (_, child) in children.iter_mut() {
                if matches!(child.try_wait(), Ok(None)) {
                    terminate_gracefully(child);
                }
            }

            // Collect exit codes.
            let statuses: Vec<(&str, i32)> = children
                .iter_mut()
                .map(|(label, child)| {
                    let code = child.wait().map(|s| s.code().unwrap_or(1)).unwrap_or(1);
                    (*label, code)
                })
                .collect();

            if ctrlc.load(Ordering::SeqCst) {
                eprintln!("\n  {} Stopped gracefully", console::style("👋").green(),);
                return ExitCode::SUCCESS;
            }

            let summary: Vec<String> = statuses
                .iter()
                .map(|(label, code)| format!("{label}={code}"))
                .collect();
            eprintln!(
                "\n  {} dev processes exited ({})",
                console::style("✘").red().bold(),
                summary.join(", "),
            );
            return ExitCode::FAILURE;
        }

        thread::sleep(Duration::from_millis(100));
    }
}

/// Signal all children gracefully, then force-kill any that don't exit in time.
fn shutdown_group(children: &mut [(&str, ManagedChild)]) {
    for (_, child) in children.iter_mut() {
        sys::send_graceful_stop(&child.inner);
    }

    let deadline = Instant::now() + GRACEFUL_TIMEOUT;
    let mut done = vec![false; children.len()];

    while Instant::now() < deadline && done.iter().any(|d| !d) {
        for (i, (_, child)) in children.iter_mut().enumerate() {
            if !done[i] {
                done[i] = matches!(child.try_wait(), Ok(Some(_)));
            }
        }
        if done.iter().any(|d| !d) {
            thread::sleep(Duration::from_millis(50));
        }
    }

    for (i, (_, child)) in children.iter_mut().enumerate() {
        if !done[i] {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// ---------------------------------------------------------------------------
// Platform-specific implementations
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod sys {
    use std::process::{Child, Command};

    /// Make the child a process-group leader so we can signal the entire tree.
    #[allow(unsafe_code)]
    pub fn configure_process_group(command: &mut Command) {
        use std::os::unix::process::CommandExt;

        // SAFETY: `setpgid(0, 0)` is async-signal-safe and makes the child its
        // own process-group leader. Called between fork and exec.
        unsafe {
            command.pre_exec(|| {
                libc::setpgid(0, 0);
                Ok(())
            });
        }
    }

    /// Send SIGTERM to the child's process group, falling back to the child PID
    /// directly if the group signal fails.
    #[allow(unsafe_code)]
    pub fn send_graceful_stop(child: &Child) {
        #[allow(clippy::cast_possible_wrap)]
        let pid = child.id() as i32;
        // SAFETY: `libc::kill` with a valid pid is safe; we only inspect the
        // return value.
        unsafe {
            if libc::kill(-pid, libc::SIGTERM) != 0 {
                libc::kill(pid, libc::SIGTERM);
            }
        }
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod sys {
    use std::ffi::c_void;
    use std::process::{Child, Command};

    /// `CREATE_NEW_PROCESS_GROUP` makes the child the root of a new process
    /// group that can receive console control events.
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

    /// `CTRL_BREAK_EVENT` can be targeted at a specific process group (unlike
    /// `CTRL_C_EVENT` which always targets the current console's group).
    const CTRL_BREAK_EVENT: u32 = 1;

    // -- Job Object constants ------------------------------------------------

    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;
    const JOB_OBJECT_BASIC_ACCOUNTING_INFORMATION: u32 = 1;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: u32 = 9;
    const PROCESS_SET_QUOTA: u32 = 0x0100;
    const PROCESS_TERMINATE: u32 = 0x0001;

    type Handle = *mut c_void;

    extern "system" {
        fn GenerateConsoleCtrlEvent(dw_ctrl_event: u32, dw_process_group_id: u32) -> i32;
        fn CreateJobObjectW(security_attributes: Handle, name: *const u16) -> Handle;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
        fn QueryInformationJobObject(
            job: Handle,
            class: u32,
            info: *mut c_void,
            len: u32,
            returned_len: *mut u32,
        ) -> i32;
        fn SetInformationJobObject(job: Handle, class: u32, info: *const c_void, len: u32) -> i32;
        fn TerminateJobObject(job: Handle, exit_code: u32) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn CloseHandle(handle: Handle) -> i32;
    }

    // -- repr(C) structs matching the Windows Job Object API ------------------

    /// `JOBOBJECT_BASIC_LIMIT_INFORMATION`. The compiler-inserted padding
    /// matches the C layout on both x86 and x86_64.
    #[repr(C)]
    #[allow(dead_code)]
    struct BasicLimitInfo {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    /// `IO_COUNTERS`.
    #[repr(C)]
    #[allow(dead_code)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }

    /// `JOBOBJECT_BASIC_ACCOUNTING_INFORMATION`.
    #[repr(C)]
    #[allow(dead_code)]
    struct BasicAccountingInfo {
        total_user_time: i64,
        total_kernel_time: i64,
        this_period_total_user_time: i64,
        this_period_total_kernel_time: i64,
        total_page_fault_count: u32,
        total_processes: u32,
        active_processes: u32,
        total_terminated_processes: u32,
    }

    /// `JOBOBJECT_EXTENDED_LIMIT_INFORMATION`.
    #[repr(C)]
    #[allow(dead_code)]
    struct ExtendedLimitInfo {
        basic: BasicLimitInfo,
        io_info: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    // -- JobHandle RAII wrapper -----------------------------------------------

    /// RAII wrapper for a Windows Job Object handle.
    ///
    /// When dropped, the handle is closed. Because the job is created with
    /// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, all processes still in the job are
    /// terminated  ensuring no orphaned children survive a `cmd.exe /c` wrapper.
    pub struct JobHandle(Handle);

    // SAFETY: Job Object handles are not bound to a specific thread.
    unsafe impl Send for JobHandle {}
    unsafe impl Sync for JobHandle {}

    impl JobHandle {
        /// Create a Job Object with `KILL_ON_JOB_CLOSE` and assign `child` to
        /// it. Returns a no-op handle if any Win32 call fails (best-effort).
        pub fn attach(child: &Child) -> Self {
            // SAFETY: `CreateJobObjectW` with null args creates an unnamed job.
            let job = unsafe { CreateJobObjectW(std::ptr::null_mut(), std::ptr::null()) };
            if job.is_null() {
                return Self(std::ptr::null_mut());
            }

            let info = ExtendedLimitInfo {
                basic: BasicLimitInfo {
                    per_process_user_time_limit: 0,
                    per_job_user_time_limit: 0,
                    limit_flags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                    minimum_working_set_size: 0,
                    maximum_working_set_size: 0,
                    active_process_limit: 0,
                    affinity: 0,
                    priority_class: 0,
                    scheduling_class: 0,
                },
                io_info: IoCounters {
                    read_operation_count: 0,
                    write_operation_count: 0,
                    other_operation_count: 0,
                    read_transfer_count: 0,
                    write_transfer_count: 0,
                    other_transfer_count: 0,
                },
                process_memory_limit: 0,
                job_memory_limit: 0,
                peak_process_memory_used: 0,
                peak_job_memory_used: 0,
            };

            // SAFETY: `info` is a valid, zero-initialised
            // `JOBOBJECT_EXTENDED_LIMIT_INFORMATION` with only `LimitFlags` set.
            unsafe {
                SetInformationJobObject(
                    job,
                    JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                    std::ptr::addr_of!(info).cast(),
                    std::mem::size_of::<ExtendedLimitInfo>() as u32,
                );
            }

            let access = PROCESS_SET_QUOTA | PROCESS_TERMINATE;
            // SAFETY: `child.id()` is a valid PID of a process we just spawned.
            let process = unsafe { OpenProcess(access, 0, child.id()) };
            if !process.is_null() {
                // SAFETY: both handles are valid.
                unsafe {
                    AssignProcessToJobObject(job, process);
                    CloseHandle(process);
                }
            }

            Self(job)
        }

        pub fn has_active_processes(&self) -> bool {
            self.active_process_count() > 0
        }

        pub fn terminate_all(&self) {
            if self.0.is_null() {
                return;
            }

            // SAFETY: `self.0` is a valid job handle and terminating the job is
            // the intended forced-shutdown path for the managed process tree.
            unsafe {
                TerminateJobObject(self.0, 1);
            }
        }

        fn active_process_count(&self) -> u32 {
            if self.0.is_null() {
                return 0;
            }

            let mut info = BasicAccountingInfo {
                total_user_time: 0,
                total_kernel_time: 0,
                this_period_total_user_time: 0,
                this_period_total_kernel_time: 0,
                total_page_fault_count: 0,
                total_processes: 0,
                active_processes: 0,
                total_terminated_processes: 0,
            };
            let mut returned_len = 0u32;

            // SAFETY: `info` points to a valid writable buffer of the requested
            // type and `self.0` is a valid job handle.
            let ok = unsafe {
                QueryInformationJobObject(
                    self.0,
                    JOB_OBJECT_BASIC_ACCOUNTING_INFORMATION,
                    std::ptr::addr_of_mut!(info).cast(),
                    std::mem::size_of::<BasicAccountingInfo>() as u32,
                    std::ptr::addr_of_mut!(returned_len),
                )
            };

            if ok == 0 {
                return 0;
            }

            info.active_processes
        }
    }

    impl Drop for JobHandle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: `self.0` is a valid Job Object handle (guarded).
                // Closing triggers `KILL_ON_JOB_CLOSE` for remaining processes.
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
    }

    pub fn configure_process_group(command: &mut Command) {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }

    /// Send `CTRL_BREAK_EVENT` to the child's process group  the Windows
    /// equivalent of Unix SIGTERM for console applications.
    pub fn send_graceful_stop(child: &Child) {
        // SAFETY: `GenerateConsoleCtrlEvent` is safe to call with a valid
        // process group id. The child was created with
        // `CREATE_NEW_PROCESS_GROUP`, so its PID doubles as its group id.
        unsafe {
            GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, child.id());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, TcpListener};

    #[test]
    fn test_ensure_reserved_ports_available_reports_port_conflict() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();

        let error =
            ensure_reserved_ports_available("demo-app", &[ReservedPort::new("server", port)])
                .unwrap_err();

        assert!(error.contains("demo-app"));
        assert!(error.contains(&format!("server={port}")));
        drop(listener);
    }

    #[cfg(windows)]
    #[test]
    fn test_try_wait_ignores_wrapper_exit_while_job_processes_are_alive() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let script_path = dir.path().join("detach.cmd");
        std::fs::write(
            &script_path,
            "@echo off\r\nstart \"\" /b powershell -NoProfile -WindowStyle Hidden -Command \"Start-Sleep -Seconds 2\"\r\nexit /b 0\r\n",
        )
        .unwrap();

        let mut command = Command::new("cmd");
        command
            .arg("/c")
            .arg(&script_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let mut managed = spawn_managed(&mut command).unwrap();

        thread::sleep(Duration::from_millis(200));
        assert!(managed.try_wait().unwrap().is_none());

        let status = managed.wait().unwrap();
        assert_eq!(status.code(), Some(0));
    }
}
