// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Wake, Waker};

use crate::execution::WorkError;

// Polls only completion/delivery futures. Application work belongs to the
// ApplicationExecutor, never this UI-local driver.
pub(crate) struct NativeTasks {
    tasks: RefCell<Vec<Task>>,
    count: Cell<usize>,
    polling: Cell<bool>,
    deferred_drain: Cell<bool>,
    alive: Arc<AtomicBool>,
    wake: Arc<dyn Fn() -> bool + Send + Sync>,
    max_tasks: usize,
}

struct Task {
    future: Pin<Box<dyn Future<Output = ()>>>,
    wake: Arc<TaskWake>,
}

struct TaskWake {
    ready: AtomicBool,
    active: AtomicBool,
    alive: Arc<AtomicBool>,
    native: Arc<dyn Fn() -> bool + Send + Sync>,
}

impl Wake for TaskWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        if self.alive.load(Ordering::Acquire)
            && self.active.load(Ordering::Acquire)
            && !self.ready.swap(true, Ordering::AcqRel)
        {
            (self.native)();
        }
    }
}

impl NativeTasks {
    #[cfg(all(test, feature = "application-ipc"))]
    pub(crate) fn count(&self) -> usize {
        self.count.get()
    }
    pub(crate) fn new(wake: Arc<dyn Fn() -> bool + Send + Sync>, max_tasks: usize) -> Self {
        Self {
            tasks: RefCell::new(Vec::new()),
            count: Cell::new(0),
            polling: Cell::new(false),
            deferred_drain: Cell::new(false),
            alive: Arc::new(AtomicBool::new(true)),
            wake,
            max_tasks,
        }
    }

    pub(crate) fn spawn(
        &self,
        future: impl Future<Output = ()> + 'static,
    ) -> Result<(), WorkError> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(WorkError::Closed);
        }
        if self.count.get() >= self.max_tasks {
            return Err(WorkError::Overloaded);
        }
        self.count.set(self.count.get() + 1);
        self.tasks.borrow_mut().push(Task {
            future: Box::pin(future),
            wake: Arc::new(TaskWake {
                ready: AtomicBool::new(true),
                active: AtomicBool::new(true),
                alive: Arc::clone(&self.alive),
                native: Arc::clone(&self.wake),
            }),
        });
        if !(self.wake)() {
            self.close();
            return Err(WorkError::Closed);
        }
        Ok(())
    }

    pub(crate) fn poll_ready(&self) {
        if !self.alive.load(Ordering::Acquire) {
            return;
        }
        if self.polling.replace(true) {
            self.deferred_drain.set(true);
            return;
        }
        let batch = std::mem::take(&mut *self.tasks.borrow_mut());
        for mut task in batch {
            if !self.alive.load(Ordering::Acquire) {
                self.count.set(self.count.get() - 1);
                continue;
            }
            let complete = if task.wake.ready.swap(false, Ordering::AcqRel) {
                let waker = Waker::from(Arc::clone(&task.wake));
                task.future
                    .as_mut()
                    .poll(&mut Context::from_waker(&waker))
                    .is_ready()
            } else {
                false
            };
            if complete || !self.alive.load(Ordering::Acquire) {
                self.count.set(self.count.get() - 1);
            } else {
                self.tasks.borrow_mut().push(task);
            }
        }
        self.polling.set(false);
        if self.deferred_drain.replace(false) && self.alive.load(Ordering::Acquire) {
            let ready = self
                .tasks
                .borrow()
                .iter()
                .any(|task| task.wake.ready.load(Ordering::Acquire));
            if ready && !(self.wake)() {
                self.close();
            }
        }
    }

    pub(crate) fn close(&self) {
        self.alive.store(false, Ordering::Release);
        self.deferred_drain.set(false);
        let tasks = std::mem::take(&mut *self.tasks.borrow_mut());
        self.count.set(self.count.get() - tasks.len());
        drop(tasks);
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        self.wake.active.store(false, Ordering::Release);
    }
}

impl Drop for NativeTasks {
    fn drop(&mut self) {
        self.close();
    }
}
