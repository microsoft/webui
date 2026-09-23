// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! A single native hello deadline, not an idle polling loop.

use super::ipc_policy::error;
use crate::{
    ipc::{IpcError, IpcErrorCode},
    native_ipc::NativeHello,
};
use std::time::{Duration, Instant};
use windows::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{KillTimer, SetTimer},
};

pub(super) struct HelloDeadline {
    pub hello: NativeHello,
    pub expires: Instant,
}

impl HelloDeadline {
    pub fn start(hwnd: HWND, cookie: usize, hello: &NativeHello) -> Result<Self, IpcError> {
        // The browser bootstrap uses this same bounded default before receiving
        // negotiated session limits. The timer exists only during native hello.
        const TIMEOUT_MS: u32 = 5000;
        // SAFETY: Called on the window's STA. No callback pointers or heap
        // payloads cross threads; WM_TIMER is handled by this window's procedure.
        if unsafe { SetTimer(Some(hwnd), cookie, TIMEOUT_MS, None) } == 0 {
            return Err(error(IpcErrorCode::Transport));
        }
        Ok(Self {
            hello: NativeHello {
                call_id: hello.call_id.clone(),
                hello: hello.hello.clone(),
                proof: hello.proof.clone(),
            },
            expires: Instant::now() + Duration::from_millis(u64::from(TIMEOUT_MS)),
        })
    }

    pub fn expired(&self, now: Instant) -> bool {
        now >= self.expires
    }
}

pub(super) fn cancel(hwnd: HWND, cookie: usize) {
    // SAFETY: The owning STA removes only this adapter's uniquely named timer.
    let _ = unsafe { KillTimer(Some(hwnd), cookie) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_old_timer_does_not_expire_new_hello() {
        let now = Instant::now();
        let deadline = HelloDeadline {
            hello: NativeHello {
                call_id: "1".into(),
                hello: crate::ipc::Hello {
                    wire_version: 2,
                    contract_name: "test".into(),
                    contract_major: 1,
                    schema_hash: "0".repeat(64),
                },
                proof: crate::ipc::DocumentActivation {
                    navigation: 2,
                    document_nonce: [2; 16],
                    challenge: [3; 16],
                },
            },
            expires: now + Duration::from_secs(5),
        };
        assert!(!deadline.expired(now));
        assert!(deadline.expired(now + Duration::from_secs(5)));
    }
}
