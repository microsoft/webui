// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Bounded modal admission, one native reservation and its logical deadline.

mod lifecycle;

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use crate::HostLifetime;

use super::{DialogCopy, DialogError, DialogOutcome};

const DEADLINE: Duration = Duration::from_secs(10);
const MAX_TIMERS: usize = 8;

struct SlotState {
    result: Option<Result<DialogOutcome, DialogError>>,
    waker: Option<Waker>,
}
struct Slot(Mutex<SlotState>);
impl Slot {
    fn complete(&self, result: Result<DialogOutcome, DialogError>) {
        let wake = if let Ok(mut state) = self.0.lock() {
            if state.result.is_some() {
                return;
            }
            state.result = Some(result);
            state.waker.take()
        } else {
            None
        };
        if let Some(wake) = wake {
            wake.wake();
        }
    }
    fn poll(&self, cx: &mut Context<'_>) -> Poll<Result<DialogOutcome, DialogError>> {
        let Ok(mut state) = self.0.lock() else {
            return Poll::Ready(Err(DialogError::Unavailable));
        };
        if let Some(value) = state.result.take() {
            return Poll::Ready(value);
        }
        state.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

struct Timer {
    finished: Mutex<bool>,
    changed: Condvar,
}
impl Timer {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            finished: Mutex::new(false),
            changed: Condvar::new(),
        })
    }
    fn finish(&self) {
        if let Ok(mut done) = self.finished.lock() {
            *done = true;
            self.changed.notify_one();
        }
    }
}

/// Shared with a picker only when the opt-in dialog feature is enabled.
pub(crate) struct ModalPermit(Arc<AtomicBool>);
impl ModalPermit {
    pub(crate) fn claim(busy: Arc<AtomicBool>) -> Result<Self, DialogError> {
        busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| DialogError::Busy)?;
        Ok(Self(busy))
    }
}
impl Drop for ModalPermit {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub(crate) struct Signal {
    pub(crate) cancelled: AtomicBool,
}
impl Signal {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            cancelled: AtomicBool::new(false),
        })
    }
}

struct Active {
    id: u64,
    epoch: u64,
    deadline: Instant,
    timed_out: bool,
    cancelled: bool,
    signal: Arc<Signal>,
    timer: Arc<Timer>,
    slot: Weak<Slot>,
    _permit: ModalPermit,
}

struct State {
    closed: bool,
    next_id: u64,
    active: Option<Active>,
}

pub(crate) struct DialogState {
    lifetime: HostLifetime,
    epoch: AtomicU64,
    modal_busy: Arc<AtomicBool>,
    state: Mutex<State>,
    timers: AtomicUsize,
    #[cfg(target_os = "macos")]
    dispatch: Mutex<Option<Arc<crate::native_services::platform::Dispatch>>>,
    #[cfg(windows)]
    target: AtomicUsize,
}

#[must_use = "await the native acknowledgement; admission is not a user response"]
pub struct DialogRequest {
    owner: Arc<DialogState>,
    slot: Arc<Slot>,
    id: u64,
    epoch: u64,
    deadline: Instant,
    delivered: bool,
}

impl DialogState {
    pub(crate) fn new(lifetime: HostLifetime, modal_busy: Arc<AtomicBool>) -> Arc<Self> {
        Arc::new(Self {
            lifetime,
            epoch: AtomicU64::new(0),
            modal_busy,
            state: Mutex::new(State {
                closed: false,
                next_id: 0,
                active: None,
            }),
            timers: AtomicUsize::new(0),
            #[cfg(target_os = "macos")]
            dispatch: Mutex::new(None),
            #[cfg(windows)]
            target: AtomicUsize::new(0),
        })
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn attach(&self, dispatch: Arc<crate::native_services::platform::Dispatch>) {
        if let Ok(mut attached) = self.dispatch.lock() {
            *attached = Some(dispatch);
        }
    }
    #[cfg(windows)]
    pub(crate) fn attach(&self, window: usize) {
        self.target.store(window, Ordering::Release);
    }

    pub(crate) fn claim_modal(&self) -> Result<ModalPermit, DialogError> {
        ModalPermit::claim(Arc::clone(&self.modal_busy))
    }

    fn is_closed(&self) -> bool {
        !self.lifetime.is_active() || self.state.lock().map_or(true, |state| state.closed)
    }

    pub(crate) fn begin(self: &Arc<Self>, copy: DialogCopy) -> Result<DialogRequest, DialogError> {
        if !self.lifetime.is_active() {
            self.close_silent();
            return Err(DialogError::Closed);
        }
        let permit = self.claim_modal()?;
        let slot = Arc::new(Slot(Mutex::new(SlotState {
            result: None,
            waker: None,
        })));
        let timer = Timer::new();
        let deadline = Instant::now() + DEADLINE;
        let signal = Signal::new();
        let epoch = self.epoch.load(Ordering::Acquire);
        let id = {
            let mut state = self.state.lock().map_err(|_| DialogError::Unavailable)?;
            if state.closed || !self.lifetime.is_active() {
                return Err(DialogError::Closed);
            }
            if state.active.is_some() {
                return Err(DialogError::Busy);
            }
            let id = state
                .next_id
                .checked_add(1)
                .ok_or(DialogError::Unavailable)?;
            state.next_id = id;
            state.active = Some(Active {
                id,
                epoch,
                deadline,
                timed_out: false,
                cancelled: false,
                signal: Arc::clone(&signal),
                timer: Arc::clone(&timer),
                slot: Arc::downgrade(&slot),
                _permit: permit,
            });
            id
        };
        if self
            .timers
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                (value < MAX_TIMERS).then_some(value + 1)
            })
            .is_err()
        {
            self.abort(id);
            return Err(DialogError::Unavailable);
        }
        let timer_copy = Arc::clone(&timer);
        let weak = Arc::downgrade(self);
        if std::thread::Builder::new()
            .name("webui-dialog-deadline".into())
            .spawn(move || {
                let Some(owner) = weak.upgrade() else { return };
                let Ok(done) = timer_copy.finished.lock() else {
                    return;
                };
                let left = deadline.saturating_duration_since(Instant::now());
                let expired = timer_copy
                    .changed
                    .wait_timeout_while(done, left, |done| !*done)
                    .is_ok_and(|(done, _)| !*done);
                if expired {
                    owner.timeout(id);
                }
                owner.timers.fetch_sub(1, Ordering::AcqRel);
            })
            .is_err()
        {
            self.timers.fetch_sub(1, Ordering::AcqRel);
            self.abort(id);
            return Err(DialogError::Unavailable);
        }
        #[cfg(target_os = "macos")]
        let submission = self
            .dispatch
            .lock()
            .map_err(|_| DialogError::Unavailable)?
            .as_ref()
            .cloned()
            .ok_or(DialogError::Unavailable)
            .and_then(|dispatch| {
                dispatch.enqueue_dialog(Arc::clone(self), (id, epoch), copy, Arc::clone(&signal))
            });
        #[cfg(windows)]
        let submission = crate::windows::dialogs::submit(
            Arc::clone(self),
            (id, epoch),
            copy,
            Arc::clone(&signal),
            self.target.load(Ordering::Acquire),
        );
        #[cfg(not(any(target_os = "macos", windows)))]
        let submission: Result<(), DialogError> = Err(DialogError::Unsupported);
        if let Err(error) = submission {
            self.abort(id);
            return Err(error);
        }
        Ok(DialogRequest {
            owner: Arc::clone(self),
            slot,
            id,
            epoch,
            deadline,
            delivered: false,
        })
    }

    fn abort(&self, id: u64) {
        if let Ok(mut state) = self.state.lock() {
            if state.active.as_ref().is_some_and(|active| active.id == id) {
                if let Some(active) = state.active.take() {
                    active.timer.finish();
                }
            }
        }
    }

    fn timeout(&self, id: u64) {
        let slot = if let Ok(mut state) = self.state.lock() {
            let Some(active) = state.active.as_mut().filter(|active| active.id == id) else {
                return;
            };
            active.timed_out = true;
            active.signal.cancelled.store(true, Ordering::Release);
            active.slot.upgrade()
        } else {
            None
        };
        #[cfg(target_os = "macos")]
        self.schedule_cancel(id);
        if let Some(slot) = slot {
            slot.complete(Err(DialogError::Timeout));
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::task::Waker;

    #[test]
    fn native_reservation_survives_timeout_and_cancel_until_completion() {
        let (host, lifetime) = HostLifetime::new();
        let owner = DialogState::new(lifetime, Arc::new(AtomicBool::new(false)));
        let permit = owner.claim_modal().unwrap();
        assert_eq!(owner.claim_modal().err(), Some(DialogError::Busy));
        drop(permit);
        let slot = Arc::new(Slot(Mutex::new(SlotState {
            result: None,
            waker: None,
        })));
        let id = 1;
        owner.modal_busy.store(true, Ordering::Release);
        owner.state.lock().unwrap().active = Some(Active {
            id,
            epoch: 0,
            deadline: Instant::now() + DEADLINE,
            timed_out: false,
            cancelled: false,
            signal: Signal::new(),
            timer: Timer::new(),
            slot: Arc::downgrade(&slot),
            _permit: ModalPermit(Arc::clone(&owner.modal_busy)),
        });
        owner.timeout(id);
        assert!(matches!(
            slot.poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Err(DialogError::Timeout))
        ));
        assert_eq!(owner.claim_modal().err(), Some(DialogError::Busy));
        owner.complete(id, Ok(DialogOutcome::Confirmed));
        assert!(owner.claim_modal().is_ok());
        host.revoke().unwrap();
    }
}
