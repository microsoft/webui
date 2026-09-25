// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::future::Future;
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc, Arc, Mutex,
};
use std::task::{Context, Poll, Waker};

const MAX_WORK: usize = 16;
const WORKERS: usize = 2;
type Job = Box<dyn FnOnce() + Send>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkError {
    Closed,
    Overloaded,
}

// One lazy application/I/O boundary per owning frame. Completion storage counts
// against the same admission limit as queued and running work.
#[derive(Default)]
pub(crate) struct ApplicationExecutor {
    sender: Mutex<Option<mpsc::SyncSender<Job>>>,
    count: Arc<AtomicUsize>,
    closed: Arc<AtomicBool>,
}

struct Permit(Arc<AtomicUsize>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

struct Slot<T> {
    result: Option<T>,
    waker: Option<Waker>,
    closed: bool,
}

struct JobCompletion<T>(std::sync::Weak<Mutex<Slot<T>>>);
impl<T> Drop for JobCompletion<T> {
    fn drop(&mut self) {
        if let Some(slot) = self.0.upgrade() {
            let wake = if let Ok(mut slot) = slot.lock() {
                slot.closed = true;
                slot.waker.take()
            } else {
                None
            };
            if let Some(wake) = wake {
                wake.wake();
            }
        }
    }
}

pub(crate) struct Completion<T> {
    slot: Arc<Mutex<Slot<T>>>,
    _permit: Arc<Permit>,
}

impl<T> Future for Completion<T> {
    type Output = Result<T, WorkError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let Ok(mut slot) = self.slot.lock() else {
            return Poll::Ready(Err(WorkError::Closed));
        };
        if let Some(value) = slot.result.take() {
            return Poll::Ready(Ok(value));
        }
        if slot.closed {
            return Poll::Ready(Err(WorkError::Closed));
        }
        slot.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl ApplicationExecutor {
    pub(crate) fn submit<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Result<Completion<T>, WorkError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(WorkError::Closed);
        }
        self.count
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_WORK).then_some(n + 1)
            })
            .map_err(|_| WorkError::Overloaded)?;
        let permit = Arc::new(Permit(Arc::clone(&self.count)));
        let slot = Arc::new(Mutex::new(Slot {
            result: None,
            waker: None,
            closed: false,
        }));
        let completion = JobCompletion(Arc::downgrade(&slot));
        let running = Arc::clone(&permit);
        let job = Box::new(move || {
            // Cancellation before execution avoids calling application code.
            if completion.0.strong_count() == 0 {
                return;
            }
            let result = work();
            let wake = completion.0.upgrade().and_then(|slot| {
                if let Ok(mut slot) = slot.lock() {
                    slot.result = Some(result);
                    slot.waker.take()
                } else {
                    None
                }
            });
            // Publish only after releasing the running-work reservation: a
            // completion may immediately submit the next file chunk.
            drop(running);
            drop(completion);
            if let Some(wake) = wake {
                wake.wake();
            }
        });
        let mut sender = self.sender.lock().map_err(|_| WorkError::Closed)?;
        if self.closed.load(Ordering::Acquire) {
            return Err(WorkError::Closed);
        }
        if sender.is_none() {
            let (tx, rx) = mpsc::sync_channel::<Job>(MAX_WORK);
            let receiver = Arc::new(Mutex::new(rx));
            for index in 0..WORKERS {
                let receiver = Arc::clone(&receiver);
                let closed = Arc::clone(&self.closed);
                std::thread::Builder::new()
                    .name(format!("webui-app-{index}"))
                    .spawn(move || loop {
                        let job = match receiver.lock() {
                            Ok(receiver) => receiver.recv(),
                            Err(_) => break,
                        };
                        match job {
                            Ok(job) if !closed.load(Ordering::Acquire) => job(),
                            _ => break,
                        }
                    })
                    .map_err(|_| WorkError::Closed)?;
            }
            *sender = Some(tx);
        }
        sender
            .as_ref()
            .ok_or(WorkError::Closed)?
            .try_send(job)
            .map_err(|_| WorkError::Overloaded)?;
        Ok(Completion {
            slot,
            _permit: permit,
        })
    }

    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::Release);
        if let Ok(mut sender) = self.sender.lock() {
            sender.take();
        }
    }
}

impl Drop for ApplicationExecutor {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn executor_is_lazy_and_bounds_queued_running_and_completed_work() {
        let executor = ApplicationExecutor::default();
        assert!(executor.sender.lock().unwrap().is_none());
        let (tx, rx) = mpsc::channel();
        let mut pending = Vec::new();
        for _ in 0..MAX_WORK {
            let tx = tx.clone();
            pending.push(
                executor
                    .submit(move || {
                        tx.send(std::thread::current().id()).unwrap();
                        42
                    })
                    .unwrap(),
            );
        }
        for _ in 0..MAX_WORK {
            assert_ne!(
                rx.recv_timeout(Duration::from_secs(5)).unwrap(),
                std::thread::current().id()
            );
        }
        assert!(matches!(executor.submit(|| 0), Err(WorkError::Overloaded)));
        drop(pending);
        assert!(executor.submit(|| 0).is_ok());
        executor.close();
        assert!(matches!(executor.submit(|| 0), Err(WorkError::Closed)));
    }

    #[test]
    fn cancelled_queued_work_does_not_run_and_close_does_not_join_callbacks() {
        let executor = ApplicationExecutor::default();
        let (started, started_rx) = mpsc::channel();
        let mut releases = Vec::new();
        let mut running = Vec::new();
        for _ in 0..WORKERS {
            let (release, wait) = mpsc::channel();
            let started = started.clone();
            running.push(
                executor
                    .submit(move || {
                        started.send(()).unwrap();
                        wait.recv().unwrap();
                    })
                    .unwrap(),
            );
            releases.push(release);
        }
        for _ in 0..WORKERS {
            started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        let called = Arc::new(AtomicBool::new(false));
        let queued_called = Arc::clone(&called);
        let queued = executor
            .submit(move || queued_called.store(true, Ordering::Release))
            .unwrap();
        drop(queued);
        executor.close();
        for release in releases {
            release.send(()).unwrap();
        }
        drop(running);
        assert!(!called.load(Ordering::Acquire));
    }
}
