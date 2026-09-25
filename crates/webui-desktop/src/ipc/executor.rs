// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{engine::Core, error::fail, IpcError, IpcErrorCode};
use futures_channel::oneshot;
use futures_util::{
    future::{BoxFuture, FutureExt, Shared},
    task::SpawnExt,
};
use std::{
    collections::BTreeMap,
    future::Future,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Condvar, Mutex, MutexGuard, Weak,
    },
    time::Instant,
};

pub(super) fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    // No user code executes under these locks. A poisoned internal lock can be
    // recovered to close the session without panicking across native callbacks.
    match value.lock() {
        Ok(value) => value,
        Err(poison) => poison.into_inner(),
    }
}

/// Cooperative cancellation, clonable independently of the owning call.
#[derive(Clone)]
pub struct Cancellation {
    state: Arc<CancelState>,
}
struct CancelState {
    cancelled: AtomicBool,
    sender: Mutex<Option<oneshot::Sender<()>>>,
    future: Shared<BoxFuture<'static, ()>>,
}
impl Default for Cancellation {
    fn default() -> Self {
        let (sender, receiver) = oneshot::channel();
        Self {
            state: Arc::new(CancelState {
                cancelled: AtomicBool::new(false),
                sender: Mutex::new(Some(sender)),
                future: async move {
                    let _ = receiver.await;
                }
                .boxed()
                .shared(),
            }),
        }
    }
}
impl Cancellation {
    /// Test without waiting.
    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }
    /// Wait without recurring timer work.
    pub fn cancelled(&self) -> BoxFuture<'static, ()> {
        self.state.future.clone().boxed()
    }
    pub(super) fn cancel(&self) {
        self.state.cancelled.store(true, Ordering::Release);
        let sender = lock(&self.state.sender).take();
        if let Some(sender) = sender {
            let _ = sender.send(());
        }
    }
}

pub(super) struct Budget {
    pub tasks: AtomicUsize,
    pub bytes: AtomicUsize,
    max_tasks: usize,
    max_bytes: usize,
}
impl Budget {
    pub fn new(max_tasks: usize, max_bytes: usize) -> Arc<Self> {
        Arc::new(Self {
            tasks: AtomicUsize::new(0),
            bytes: AtomicUsize::new(0),
            max_tasks,
            max_bytes,
        })
    }
    pub fn reserve(self: &Arc<Self>, tasks: usize, bytes: usize) -> Result<Permit, IpcError> {
        add_bounded(&self.tasks, tasks, self.max_tasks)?;
        if let Err(error) = add_bounded(&self.bytes, bytes, self.max_bytes) {
            self.tasks.fetch_sub(tasks, Ordering::AcqRel);
            return Err(error);
        }
        Ok(Permit {
            budget: Arc::clone(self),
            tasks,
            bytes,
        })
    }
}
fn add_bounded(value: &AtomicUsize, count: usize, max: usize) -> Result<(), IpcError> {
    value
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |old| {
            old.checked_add(count).filter(|new| *new <= max)
        })
        .map(|_| ())
        .map_err(|_| fail(IpcErrorCode::Overloaded))
}
pub(super) struct Permit {
    budget: Arc<Budget>,
    tasks: usize,
    bytes: usize,
}
impl Permit {
    pub fn grow(&mut self, bytes: usize) -> Result<(), IpcError> {
        add_bounded(&self.budget.bytes, bytes, self.budget.max_bytes)?;
        self.bytes += bytes;
        Ok(())
    }
    pub fn shrink(&mut self, bytes: usize) {
        self.bytes -= bytes;
        self.budget.bytes.fetch_sub(bytes, Ordering::AcqRel);
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.budget.tasks.fetch_sub(self.tasks, Ordering::AcqRel);
        self.budget.bytes.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

pub(super) struct Workers {
    pool: futures_executor::ThreadPool,
}
impl Workers {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(threads: usize) -> Result<Self, IpcError> {
        futures_executor::ThreadPoolBuilder::new()
            .pool_size(threads)
            .name_prefix("webui-ipc-")
            .create()
            .map(|pool| Self { pool })
            .map_err(|_| fail(IpcErrorCode::Transport))
    }
    #[cfg(target_arch = "wasm32")]
    pub fn new(_: usize) -> Result<Self, IpcError> {
        Err(IpcError::new(
            IpcErrorCode::Transport,
            "desktop IPC workers are unavailable on wasm32",
            "run this host on a native desktop target",
        ))
    }
    pub fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) -> Result<(), IpcError> {
        self.pool
            .spawn(task)
            .map_err(|_| fail(IpcErrorCode::Transport))
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct TimerKey {
    pub generation: u64,
    pub id: u64,
    pub local: bool,
}
#[derive(Default)]
struct TimerState {
    stopped: bool,
    deadlines: BTreeMap<(Instant, TimerKey), ()>,
}
pub(super) struct Timer {
    state: Mutex<TimerState>,
    changed: Condvar,
    pub wakes: AtomicUsize,
}
impl Timer {
    pub fn new(core: Weak<Core>) -> Result<Arc<Self>, IpcError> {
        let timer = Arc::new(Self {
            state: Mutex::new(TimerState::default()),
            changed: Condvar::new(),
            wakes: AtomicUsize::new(0),
        });
        let worker = Arc::clone(&timer);
        std::thread::Builder::new()
            .name("webui-ipc-deadlines".into())
            .spawn(move || worker.run(core))
            .map_err(|_| fail(IpcErrorCode::Transport))?;
        Ok(timer)
    }
    pub fn insert(&self, deadline: Instant, key: TimerKey) {
        lock(&self.state).deadlines.insert((deadline, key), ());
        self.changed.notify_one();
    }
    pub fn remove(&self, deadline: Instant, key: TimerKey) {
        lock(&self.state).deadlines.remove(&(deadline, key));
        self.changed.notify_one();
    }
    pub fn stop(&self) {
        let mut state = lock(&self.state);
        state.stopped = true;
        state.deadlines.clear();
        self.changed.notify_one();
    }
    fn run(&self, core: Weak<Core>) {
        let mut state = lock(&self.state);
        loop {
            if state.stopped {
                return;
            }
            let Some((&(deadline, key), _)) = state.deadlines.first_key_value() else {
                state = match self.changed.wait(state) {
                    Ok(state) => state,
                    Err(error) => error.into_inner(),
                };
                continue;
            };
            let now = Instant::now();
            if deadline > now {
                state = match self.changed.wait_timeout(state, deadline - now) {
                    Ok((state, _)) => state,
                    Err(error) => error.into_inner().0,
                };
                continue;
            }
            state.deadlines.remove(&(deadline, key));
            drop(state);
            self.wakes.fetch_add(1, Ordering::Relaxed);
            if let Some(core) = core.upgrade() {
                core.timeout(key);
            } else {
                return;
            }
            state = lock(&self.state);
        }
    }
}
