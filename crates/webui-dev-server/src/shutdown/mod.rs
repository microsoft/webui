// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Internal CLI orchestration for opt-in, process-bounded dev-server shutdown.
//!
//! Call [`prepare`] before starting application threads, producing output, or
//! creating writers. The child is the same executable with unchanged arguments.
//! Only the controlled CLI may attest completed HTTP/watcher/rebuild teardown
//! with [`JOINED_EXIT_CODE`]; arbitrary programs are not supported.

mod control;
mod error;
#[cfg(unix)]
#[path = "unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "windows.rs"]
mod platform;
mod supervisor;
mod tree;
#[cfg(unix)]
mod unix_signals;

pub use control::Control;
pub use error::{Error, ForcedReason};

use std::num::NonZeroU64;
use std::process::Command;
use std::time::{Duration, Instant};

const CHILD_ENV: &str = "_WEBUI_DEV_SERVER_CHILD";
const CHILD_VERSION: &str = "joined-v1";
const CONFIRMATION: Duration = Duration::from_secs(2);
const RUNNING_POLL: Duration = Duration::from_millis(100);
const STOPPING_POLL: Duration = Duration::from_millis(10);

/// Private success attestation, used only after HTTP and rebuild teardown joined.
pub const JOINED_EXIT_CODE: i32 = 125;

/// Whether this invocation serves directly, serves under supervision, or exits.
pub enum Mode {
    /// No shutdown policy: retain the existing in-process signal/lifetime behavior.
    Direct,
    /// A contained worker whose HTTP server must disable its own signal handlers.
    Child(Control),
    /// The parent confirmed termination; exit with this code without re-reporting.
    Supervisor(i32),
}

/// Enter the private startup gate or supervise this invocation of the CLI.
///
/// `timeout` is the graceful shutdown budget in seconds. The separate process
/// termination confirmation budget is two seconds. Neither is a hard real-time
/// promise: operating-system calls and scheduling can take additional time.
///
/// Call before application threads, output, extraction, builds, or watchers.
/// A child reserves stdin for control and removes its private marker before
/// starting any threads so build subprocesses do not inherit that marker.
/// Without a marker or timeout this installs no handler and spawns nothing.
///
/// Bounded serving treats every supported stop signal alike: HTTP connections
/// stop immediately (`stop(false)`), then active rebuild work joins within the
/// configured budget. Unix SIGINT, SIGTERM, SIGHUP, and SIGQUIT all request this
/// policy. This deliberately differs from default Actix's graceful SIGTERM
/// HTTP shutdown; the no-timeout path keeps Actix's existing signal behavior.
///
/// # Errors
///
/// Errors do not authorize deleting output files. In particular, an unconfirmed
/// termination requires investigating remaining processes before cleanup.
pub fn prepare(timeout: Option<NonZeroU64>) -> Result<Mode, Error> {
    if let Some(control) = control::child_gate()? {
        return Ok(Mode::Child(control));
    }
    let Some(seconds) = timeout else {
        return Ok(Mode::Direct);
    };
    let grace = Duration::from_secs(seconds.get());
    validate_policy(grace)?;
    let (stop, requests) = supervisor::stop_channel();
    ctrlc::set_handler(move || {
        let _ = stop.request();
    })
    .map_err(Error::Signal)?;
    #[cfg(unix)]
    let mut quit = unix_signals::QuitForwarder::install()?;
    let result = std::env::current_exe()
        .map_err(Error::Startup)
        .and_then(|executable| {
            let mut command = Command::new(executable);
            command.args(std::env::args_os().skip(1));
            supervisor::supervise(command, grace, requests)
        });
    #[cfg(unix)]
    let restored = quit.restore();
    // Restoration runs even on supervision failure, but never replaces a
    // potentially unconfirmed process-tree termination with a lesser error.
    let code = result?;
    #[cfg(unix)]
    restored?;
    Ok(Mode::Supervisor(code))
}

fn validate_policy(grace: Duration) -> Result<(), Error> {
    let now = Instant::now();
    if grace.is_zero()
        || now.checked_add(grace).is_none()
        || now.checked_add(CONFIRMATION).is_none()
    {
        return Err(Error::InvalidPolicy);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
