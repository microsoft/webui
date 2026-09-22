// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Attach the Windows command wakeup only to an installed native receiver.

#[cfg(all(test, not(windows)))]
#[path = "ipc_body.rs"]
mod ipc_body_contract;

use anyhow::{Context, Result};

#[cfg(windows)]
use crate::ipc::IpcWake;
use crate::ipc::{IpcError, IpcErrorCode};
use crate::WindowHandle;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
#[cfg(windows)]
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::PostMessageW,
};

static NEXT_COOKIE: AtomicUsize = AtomicUsize::new(1);

/// Closing and posting serialize under one short lock. A queued wake also
/// carries a non-reusable cookie so HWND reuse cannot drain another window.
pub(super) struct IpcWindowWake {
    target: Mutex<WakeTarget>,
    pub cookie: usize,
}

struct WakeTarget {
    hwnd: Option<usize>,
    pending: bool,
}

impl IpcWindowWake {
    #[cfg(windows)]
    pub fn new(hwnd: HWND) -> Result<Self, IpcError> {
        Self::for_handle(hwnd.0 as usize)
    }

    // Keep the coalescing/liveness policy independent of Win32 so both native
    // backends can run its regression without constructing a window.
    fn for_handle(handle: usize) -> Result<Self, IpcError> {
        let cookie = NEXT_COOKIE
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| wake_error(IpcErrorCode::Closed))?;
        Ok(Self {
            target: Mutex::new(WakeTarget {
                hwnd: Some(handle),
                pending: false,
            }),
            cookie,
        })
    }

    pub fn take(&self, cookie: usize) -> bool {
        let Ok(mut target) = self.target.lock() else {
            return false;
        };
        if cookie != self.cookie || target.hwnd.is_none() {
            return false;
        }
        target.pending = false;
        true
    }

    pub fn close(&self) {
        if let Ok(mut target) = self.target.lock() {
            target.hwnd = None;
            target.pending = false;
        }
    }

    fn post(
        &self,
        post: impl FnOnce(usize, usize) -> Result<(), IpcError>,
    ) -> Result<(), IpcError> {
        let mut target = self
            .target
            .lock()
            .map_err(|_| wake_error(IpcErrorCode::Closed))?;
        let hwnd = target
            .hwnd
            .ok_or_else(|| wake_error(IpcErrorCode::Closed))?;
        if !target.pending {
            post(hwnd, self.cookie)?;
            target.pending = true;
        }
        Ok(())
    }
}

#[cfg(windows)]
impl IpcWake for IpcWindowWake {
    fn wake(&self) -> Result<(), IpcError> {
        self.post(|handle, cookie| {
            // SAFETY: PostMessageW is thread-safe, carries no pointers, and the
            // target is invalidated under this same lock before window teardown.
            unsafe {
                PostMessageW(
                    Some(HWND(handle as *mut std::ffi::c_void)),
                    super::IPC_WAKE_MESSAGE,
                    WPARAM(cookie),
                    LPARAM(0),
                )
            }
            .map_err(|_| wake_error(IpcErrorCode::Transport))
        })
    }
}

#[cold]
#[inline(never)]
fn wake_error(code: IpcErrorCode) -> IpcError {
    IpcError::new(
        code,
        "native IPC wake is unavailable",
        "use a live desktop window and its installed native message receiver",
    )
}

pub(super) fn attach<F>(receiver: Option<WindowHandle>, wake: F) -> Result<()>
where
    F: Fn() + Send + Sync + 'static,
{
    let receiver = receiver.context(
        "cannot attach a Windows command wakeup before FrameState is installed; attach it after set_window_state",
    )?;
    receiver.set_wakeup(wake);
    Ok(())
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use crate::{WindowCommand, WindowHandle};

    #[test]
    fn ipc_wake_coalesces_rejects_stale_cookie_and_stops_at_close() {
        let wake = super::IpcWindowWake::for_handle(0).unwrap();
        let count = AtomicUsize::new(0);
        for _ in 0..50 {
            wake.post(|_, _| {
                count.fetch_add(1, Ordering::Relaxed);
                Ok(())
            })
            .unwrap();
        }
        assert_eq!(count.load(Ordering::Relaxed), 1);
        assert!(!wake.take(wake.cookie + 1));
        assert!(wake.take(wake.cookie));
        wake.post(|_, _| {
            count.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })
        .unwrap();
        assert_eq!(count.load(Ordering::Relaxed), 2);
        wake.close();
        assert!(!wake.take(wake.cookie));
        assert!(wake
            .post(|_, _| panic!("closed windows must not be posted"))
            .is_err());
    }

    #[test]
    fn receiver_guard_preserves_backlog_until_attachment_and_drain() {
        let handle = WindowHandle::default();
        let pending_wakes = Arc::new(AtomicUsize::new(0));
        handle.set_title("queued before initialization").unwrap();

        let wake_count = Arc::clone(&pending_wakes);
        assert!(super::attach(None, move || {
            wake_count.fetch_add(1, Ordering::SeqCst);
        })
        .is_err());
        // A nested initialization pump has no wake to consume before a receiver
        // exists. The production channel must retain both accepted commands.
        assert_eq!(pending_wakes.swap(0, Ordering::SeqCst), 0);
        handle.minimize().unwrap();
        assert_eq!(pending_wakes.load(Ordering::SeqCst), 0);

        let wake_count = Arc::clone(&pending_wakes);
        super::attach(Some(handle.clone()), move || {
            wake_count.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        assert_eq!(pending_wakes.swap(0, Ordering::SeqCst), 1);
        let commands = handle.drain_commands();
        assert_eq!(commands.len(), 2);
        assert!(matches!(
            &commands[0],
            WindowCommand::SetTitle(title) if title == "queued before initialization"
        ));
        assert!(matches!(commands[1], WindowCommand::Minimize));

        handle.set_title("queued after startup drain").unwrap();
        assert_eq!(pending_wakes.load(Ordering::SeqCst), 1);
    }
}
