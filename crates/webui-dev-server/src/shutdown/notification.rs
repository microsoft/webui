// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::Error;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone)]
pub(super) struct Notification(SyncSender<()>);

impl Notification {
    pub(super) fn wake(&self) -> bool {
        match self.0.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) => true,
            Err(TrySendError::Disconnected(())) => false,
        }
    }
}

#[derive(Default)]
struct StopState {
    requests: AtomicU8,
    disconnected: AtomicBool,
}

pub(super) struct StopHandle {
    state: Arc<StopState>,
    notification: Notification,
}

impl StopState {
    fn request(&self, notification: &Notification) -> bool {
        // The channel is only a wakeup, not the source of truth. Saturating at
        // two preserves force-stop even if every notification coalesces.
        let _ = self
            .requests
            .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                Some(count.saturating_add(1).min(2))
            });
        notification.wake()
    }
}

impl StopHandle {
    pub(super) fn request(&self) -> bool {
        self.state.request(&self.notification)
    }
}

impl Drop for StopHandle {
    fn drop(&mut self) {
        self.state.disconnected.store(true, Ordering::Release);
        self.notification.wake();
    }
}

pub(super) struct Requests {
    state: Arc<StopState>,
    notifications: Receiver<()>,
    pub(super) notification: Notification,
}

impl Requests {
    pub(super) fn request(&self) -> bool {
        self.state.request(&self.notification)
    }

    pub(super) fn count(&self) -> u8 {
        self.state.requests.load(Ordering::Acquire)
    }

    pub(super) fn disconnected(&self) -> bool {
        self.state.disconnected.load(Ordering::Acquire)
    }

    pub(super) fn wait(&self, deadline: Option<Instant>) -> Result<bool, Error> {
        let result = match deadline {
            Some(limit) => self
                .notifications
                .recv_timeout(limit.saturating_duration_since(Instant::now())),
            None => self
                .notifications
                .recv()
                .map_err(|_| RecvTimeoutError::Disconnected),
        };
        match result {
            Ok(()) => Ok(true),
            Err(RecvTimeoutError::Timeout) => Ok(false),
            Err(RecvTimeoutError::Disconnected) => Err(Error::Control(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "supervisor notification channel closed",
            ))),
        }
    }
}

pub(super) fn stop_channel() -> (StopHandle, Requests) {
    let (sender, notifications) = mpsc::sync_channel(1);
    let state = Arc::new(StopState::default());
    let notification = Notification(sender);
    (
        StopHandle {
            state: Arc::clone(&state),
            notification: notification.clone(),
        },
        Requests {
            state,
            notifications,
            notification,
        },
    )
}
