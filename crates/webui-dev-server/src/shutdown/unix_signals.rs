// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(unsafe_code)]

use super::Error;
use std::io;
use std::mem::zeroed;
use std::sync::atomic::{AtomicBool, Ordering};

static FORWARD_FAILED: AtomicBool = AtomicBool::new(false);

// `ctrlc` handles INT/TERM/HUP but not Actix's other stop signal, QUIT.
pub(super) struct QuitForwarder {
    previous: Option<libc::sigaction>,
}

impl QuitForwarder {
    // Install only after ctrlc's SIGTERM handler, before spawning any child.
    pub(super) fn install() -> Result<Self, Error> {
        // SAFETY: sigaction is a C structure permitting zero initialization.
        let mut action: libc::sigaction = unsafe { zeroed() };
        action.sa_sigaction = forward_quit as *const () as libc::sighandler_t;
        action.sa_flags = libc::SA_RESTART;
        // SAFETY: action owns a valid writable sigset_t.
        if unsafe { libc::sigemptyset(&mut action.sa_mask) } != 0 {
            return Err(signal_error(io::Error::last_os_error()));
        }
        // SAFETY: sigaction is an output C structure permitting zero initialization.
        let mut previous: libc::sigaction = unsafe { zeroed() };
        FORWARD_FAILED.store(false, Ordering::Relaxed);
        // SAFETY: SIGQUIT is valid; both structures cover their full C layout.
        // Only this parent owns registration during the prepare invocation.
        if unsafe { libc::sigaction(libc::SIGQUIT, &action, &mut previous) } != 0 {
            return Err(signal_error(io::Error::last_os_error()));
        }
        Ok(Self {
            previous: Some(previous),
        })
    }

    pub(super) fn restore(&mut self) -> Result<(), Error> {
        let Some(previous) = self.previous.take() else {
            return Ok(());
        };
        // SAFETY: previous was initialized by sigaction and retains its complete
        // handler, mask, and flags. The child cleanup attempt already finished.
        if unsafe { libc::sigaction(libc::SIGQUIT, &previous, std::ptr::null_mut()) } != 0 {
            return Err(signal_error(io::Error::last_os_error()));
        }
        Ok(())
    }
}

impl Drop for QuitForwarder {
    fn drop(&mut self) {
        if let Err(error) = self.restore() {
            // Supervision's Tree is already dropped before this outer guard.
            eprintln!(
                "Dev-server signal restoration failed: {error}; {}",
                error.help()
            );
        }
    }
}

extern "C" fn forward_quit(_signal: libc::c_int) {
    // SAFETY: POSIX raise is async-signal-safe, SIGTERM is valid, and its ctrlc
    // handler is installed before this handler. No allocation, locks, channels,
    // formatting, or process-name lookup occurs in this signal context.
    if unsafe { libc::raise(libc::SIGTERM) } != 0 {
        // AtomicBool is lock-free; the normal monitor observes this at its next
        // wake/poll and performs checked tree cleanup before reporting failure.
        FORWARD_FAILED.store(true, Ordering::Relaxed);
    }
}

pub(super) fn check_forwarding() -> io::Result<()> {
    if FORWARD_FAILED.swap(false, Ordering::Relaxed) {
        return Err(forwarding_error());
    }
    Ok(())
}

#[cold]
#[inline(never)]
fn forwarding_error() -> io::Error {
    io::Error::other("could not forward SIGQUIT to the dev-server shutdown handler")
}

#[cold]
#[inline(never)]
fn signal_error(error: io::Error) -> Error {
    Error::Signal(ctrlc::Error::System(error))
}

#[cfg(test)]
pub(super) fn test_forwarding_and_restoration() -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;
    use std::time::Duration;

    static RESTORED: AtomicUsize = AtomicUsize::new(0);
    extern "C" fn previous_handler(_signal: libc::c_int) {
        RESTORED.fetch_add(1, Ordering::Relaxed);
    }
    // This helper runs only inside a dedicated gated self-spawn fixture, never
    // in the parallel test-runner process. Its prior handler replaces SIG_DFL
    // so a failed restore assertion cannot dump core or kill the test runner.
    // SAFETY: sigaction is a C structure permitting zero initialization.
    let mut previous: libc::sigaction = unsafe { zeroed() };
    previous.sa_sigaction = previous_handler as *const () as libc::sighandler_t;
    // SAFETY: previous owns a valid writable sigset_t.
    if unsafe { libc::sigemptyset(&mut previous.sa_mask) } != 0 {
        return Err(io::Error::last_os_error().into());
    }
    // SAFETY: This isolated fixture owns its process-wide SIGQUIT disposition.
    if unsafe { libc::sigaction(libc::SIGQUIT, &previous, std::ptr::null_mut()) } != 0 {
        return Err(io::Error::last_os_error().into());
    }
    let (sender, requests) = mpsc::sync_channel(2);
    ctrlc::set_handler(move || {
        let _ = sender.try_send(());
    })?;
    let mut guard = QuitForwarder::install()?;
    for _ in 0..2 {
        // SAFETY: Valid signal, delivered only to the current isolated fixture.
        if unsafe { libc::raise(libc::SIGQUIT) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        requests.recv_timeout(Duration::from_secs(2))?;
        check_forwarding()?;
    }
    assert_eq!(RESTORED.load(Ordering::Relaxed), 0);
    guard.restore()?;
    // SAFETY: The isolated fixture's known atomic-only handler was restored.
    if unsafe { libc::raise(libc::SIGQUIT) } != 0 {
        return Err(io::Error::last_os_error().into());
    }
    assert_eq!(RESTORED.load(Ordering::Relaxed), 1);
    assert!(requests.try_recv().is_err());
    Ok(())
}
