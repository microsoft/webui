//! Cross-platform child-process lifecycle management.
//!
//! Provides spawning children in their own process group and graceful shutdown
//! via SIGTERM (Unix) or `CTRL_BREAK_EVENT` (Windows), with a timed fallback
//! to a forced kill.

use std::path::Path;
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Maximum time to wait for a child to exit after a graceful stop signal before
/// escalating to a forced kill.
const GRACEFUL_TIMEOUT: Duration = Duration::from_secs(3);

// ---------------------------------------------------------------------------
// Spawning
// ---------------------------------------------------------------------------

/// Spawn a labelled child process in its own process group.
///
/// The child inherits stdin/stdout/stderr so its output appears inline. Returns
/// `None` (and prints an error) if the process cannot be started.
pub fn spawn_child(label: &str, cmd: &str, args: &[&str], cwd: &Path) -> Option<Child> {
    eprintln!(
        "  {} starting {}",
        console::style("→").dim(),
        console::style(label).cyan().bold(),
    );

    // On Windows, non-.exe commands (e.g. .cmd/.bat scripts like pnpm) must be
    // launched through cmd.exe because CreateProcessW only resolves .exe files.
    let mut command = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/c").arg(cmd).args(args);
        c
    } else {
        let mut c = Command::new(cmd);
        c.args(args);
        c
    };
    command
        .current_dir(cwd)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    sys::configure_process_group(&mut command);

    match command.spawn() {
        Ok(child) => Some(child),
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
// Graceful shutdown
// ---------------------------------------------------------------------------

/// Send a graceful stop signal, wait up to [`GRACEFUL_TIMEOUT`], then force
/// kill.
pub fn terminate_gracefully(child: &mut Child) {
    sys::send_graceful_stop(child);

    let deadline = Instant::now() + GRACEFUL_TIMEOUT;
    while Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }

    let _ = child.kill();
    let _ = child.wait();
}

/// Run a Ctrl+C–aware poll loop for exactly two child processes.
///
/// * On Ctrl+C both children receive a graceful stop signal in parallel.
/// * If either child exits on its own the other is also stopped.
/// * Returns [`ExitCode::SUCCESS`] for user-initiated stops, or
///   [`ExitCode::FAILURE`] if a child crashed.
pub fn wait_for_pair(server: &mut Child, client: &mut Child) -> ExitCode {
    let ctrlc = Arc::new(AtomicBool::new(false));
    let flag = ctrlc.clone();
    ctrlc::set_handler(move || {
        flag.store(true, Ordering::SeqCst);
    })
    .expect("failed to set Ctrl+C handler");

    loop {
        if ctrlc.load(Ordering::SeqCst) {
            shutdown_pair(server, client);
            eprintln!("\n  {} stopped", console::style("✔").green());
            return ExitCode::SUCCESS;
        }

        let server_done = matches!(server.try_wait(), Ok(Some(_)));
        let client_done = matches!(client.try_wait(), Ok(Some(_)));

        if server_done || client_done {
            if !server_done {
                terminate_gracefully(server);
            }
            if !client_done {
                terminate_gracefully(client);
            }
            let s = server.wait().map(|s| s.code().unwrap_or(1)).unwrap_or(1);
            let c = client.wait().map(|s| s.code().unwrap_or(1)).unwrap_or(1);

            if ctrlc.load(Ordering::SeqCst) {
                eprintln!("\n  {} stopped", console::style("✔").green());
                return ExitCode::SUCCESS;
            }

            eprintln!(
                "  {} dev processes exited (server={}, client={})",
                console::style("✘").red().bold(),
                s,
                c,
            );
            return ExitCode::FAILURE;
        }

        thread::sleep(Duration::from_millis(100));
    }
}

/// Signal both children gracefully, then force-kill any that don't exit in
/// time.
fn shutdown_pair(a: &mut Child, b: &mut Child) {
    sys::send_graceful_stop(a);
    sys::send_graceful_stop(b);

    let deadline = Instant::now() + GRACEFUL_TIMEOUT;

    let mut a_done = false;
    let mut b_done = false;
    while Instant::now() < deadline && !(a_done && b_done) {
        if !a_done {
            a_done = matches!(a.try_wait(), Ok(Some(_)));
        }
        if !b_done {
            b_done = matches!(b.try_wait(), Ok(Some(_)));
        }
        if !(a_done && b_done) {
            thread::sleep(Duration::from_millis(50));
        }
    }

    if !a_done {
        let _ = a.kill();
        let _ = a.wait();
    }
    if !b_done {
        let _ = b.kill();
        let _ = b.wait();
    }
}

// ---------------------------------------------------------------------------
// Platform-specific implementations
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod sys {
    use std::process::{Child, Command};

    /// Make the child a process-group leader so we can signal the entire tree.
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
mod sys {
    use std::process::{Child, Command};

    /// `CREATE_NEW_PROCESS_GROUP` makes the child the root of a new process
    /// group that can receive console control events.
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

    /// `CTRL_BREAK_EVENT` can be targeted at a specific process group (unlike
    /// `CTRL_C_EVENT` which always targets the current console's group).
    const CTRL_BREAK_EVENT: u32 = 1;

    extern "system" {
        fn GenerateConsoleCtrlEvent(dw_ctrl_event: u32, dw_process_group_id: u32) -> i32;
    }

    pub fn configure_process_group(command: &mut Command) {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }

    /// Send `CTRL_BREAK_EVENT` to the child's process group — the Windows
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
