// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::notification::Notification;
use super::{Error, STOPPING_POLL};
use std::io;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::Instant;

/// One result is published before its wakeup; neither can be lost when wakeups
/// coalesce. No worker holds a lock while waiting for a process or output.
pub(super) struct Task<T> {
    result: Receiver<T>,
    thread: Option<JoinHandle<()>>,
    received: bool,
    name: &'static str,
}

impl<T: Send + 'static> Task<T> {
    pub(super) fn spawn(
        name: &'static str,
        notification: Notification,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> io::Result<Self> {
        let (sender, result) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                // Also wake on an unwinding thread failure, so an idle supervisor
                // reports a disconnected result channel rather than sleeping forever.
                struct Completion<T> {
                    sender: Option<SyncSender<T>>,
                    notification: Notification,
                }
                impl<T> Drop for Completion<T> {
                    fn drop(&mut self) {
                        // Disconnect before waking, including during unwinding.
                        self.sender.take();
                        self.notification.wake();
                    }
                }
                let completion = Completion {
                    sender: Some(sender),
                    notification,
                };
                let value = work();
                if let Some(sender) = &completion.sender {
                    let _ = sender.send(value);
                }
            })?;
        Ok(Self {
            result,
            thread: Some(thread),
            received: false,
            name,
        })
    }

    pub(super) fn take_result(&mut self) -> io::Result<Option<T>> {
        if self.received {
            return Ok(None);
        }
        match self.result.try_recv() {
            Ok(value) => {
                self.received = true;
                Ok(Some(value))
            }
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                self.received = true;
                Err(io::Error::other(format!(
                    "{} exited without reporting a result",
                    self.name
                )))
            }
        }
    }

    #[cold]
    pub(super) fn failure(&self, error: io::Error) -> Error {
        Error::Control(io::Error::new(
            error.kind(),
            format!("{}: {error}", self.name),
        ))
    }

    pub(super) fn join_until(&mut self, deadline: Instant) -> Result<(), Error> {
        let Some(thread) = self.thread.as_ref() else {
            return Ok(());
        };
        while !thread.is_finished() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::Control(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "{} did not finish within teardown confirmation; check for a blocked output consumer or worker",
                        self.name
                    ),
                )));
            }
            thread::sleep(remaining.min(STOPPING_POLL));
        }
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| Error::Control(io::Error::other(format!("{} panicked", self.name))))?;
        }
        Ok(())
    }
}
