// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! OS acknowledgement, cancellation and future delivery after native work.

use std::future::Future;
use std::pin::Pin;

use super::*;

impl DialogState {
    pub(crate) fn current(&self, id: u64, epoch: u64) -> bool {
        self.lifetime.is_active()
            && self.epoch.load(Ordering::Acquire) == epoch
            && self.state.lock().is_ok_and(|state| {
                !state.closed
                    && state.active.as_ref().is_some_and(|active| {
                        active.id == id
                            && active.epoch == epoch
                            && !active.cancelled
                            && !active.timed_out
                            && Instant::now() < active.deadline
                    })
            })
    }

    pub(crate) fn complete(&self, id: u64, result: Result<DialogOutcome, DialogError>) {
        let active = if let Ok(mut state) = self.state.lock() {
            if state.active.as_ref().is_none_or(|active| active.id != id) {
                return;
            }
            state.active.take()
        } else {
            return;
        };
        let Some(active) = active else { return };
        active.timer.finish();
        let result = if self.is_closed() {
            Err(DialogError::Closed)
        } else if active.cancelled || self.epoch.load(Ordering::Acquire) != active.epoch {
            Err(DialogError::Navigated)
        } else if active.timed_out || Instant::now() >= active.deadline {
            Err(DialogError::Timeout)
        } else {
            result
        };
        let slot = active.slot.upgrade();
        drop(active); // release modal admission before waking any host Future
        if let Some(slot) = slot {
            slot.complete(result);
        }
    }

    pub(crate) fn navigate(&self) {
        if self
            .epoch
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |epoch| {
                epoch.checked_add(1)
            })
            .is_err()
        {
            self.notify_closed();
            return;
        }
        let active = self.state.lock().ok().and_then(|mut state| {
            state.active.as_mut().map(|active| {
                active.cancelled = true;
                active.signal.cancelled.store(true, Ordering::Release);
                (active.id, active.slot.upgrade())
            })
        });
        if let Some((id, slot)) = active {
            #[cfg(target_os = "macos")]
            self.schedule_cancel(id);
            #[cfg(not(target_os = "macos"))]
            let _ = id;
            if let Some(slot) = slot {
                slot.complete(Err(DialogError::Navigated));
            }
        }
    }

    #[cfg(target_os = "macos")]
    pub(super) fn schedule_cancel(&self, id: u64) {
        if let Ok(dispatch) = self.dispatch.lock() {
            if let Some(dispatch) = dispatch.as_ref() {
                dispatch.cancel_dialog(id);
            }
        }
    }

    pub(crate) fn close_silent(&self) {
        let id = if let Ok(mut state) = self.state.lock() {
            state.closed = true;
            if let Some(active) = state.active.as_mut() {
                active.cancelled = true;
                active.signal.cancelled.store(true, Ordering::Release);
            }
            state.active.as_ref().map(|active| active.id)
        } else {
            None
        };
        #[cfg(target_os = "macos")]
        if let Some(id) = id {
            self.schedule_cancel(id);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = id;
    }

    pub(crate) fn notify_closed(&self) {
        self.close_silent();
        let active = self.state.lock().ok().and_then(|state| {
            state
                .active
                .as_ref()
                .map(|active| (active.id, active.slot.upgrade()))
        });
        if let Some((id, slot)) = active {
            #[cfg(target_os = "macos")]
            self.schedule_cancel(id);
            #[cfg(not(target_os = "macos"))]
            let _ = id;
            if let Some(slot) = slot {
                slot.complete(Err(DialogError::Closed));
            }
        }
    }

    fn abandon(&self, id: u64) {
        let matching = self.state.lock().is_ok_and(|mut state| {
            if let Some(active) = state.active.as_mut().filter(|active| active.id == id) {
                active.cancelled = true;
                active.signal.cancelled.store(true, Ordering::Release);
                true
            } else {
                false
            }
        });
        #[cfg(target_os = "macos")]
        if matching {
            self.schedule_cancel(id);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = matching;
    }
}

impl Future for DialogRequest {
    type Output = Result<DialogOutcome, DialogError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.owner.is_closed() {
            this.owner.close_silent();
            return Poll::Ready(Err(DialogError::Closed));
        }
        if this.owner.epoch.load(Ordering::Acquire) != this.epoch {
            return Poll::Ready(Err(DialogError::Navigated));
        }
        if Instant::now() >= this.deadline {
            this.owner.timeout(this.id);
        }
        let result = this.slot.poll(cx);
        if result.is_ready() {
            this.delivered = true;
        }
        match result {
            Poll::Ready(Ok(_)) if this.owner.is_closed() => Poll::Ready(Err(DialogError::Closed)),
            Poll::Ready(Ok(_)) if this.owner.epoch.load(Ordering::Acquire) != this.epoch => {
                Poll::Ready(Err(DialogError::Navigated))
            }
            Poll::Ready(Ok(_)) if Instant::now() >= this.deadline => {
                Poll::Ready(Err(DialogError::Timeout))
            }
            other => other,
        }
    }
}
impl Drop for DialogRequest {
    fn drop(&mut self) {
        if !self.delivered {
            self.owner.abandon(self.id);
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::task::Waker;

    #[test]
    fn native_success_cannot_be_delivered_after_window_retirement() {
        let (_host, lifetime) = HostLifetime::new();
        let owner = DialogState::new(lifetime, Arc::new(AtomicBool::new(false)));
        let slot = Arc::new(Slot(Mutex::new(SlotState {
            result: None,
            waker: None,
        })));
        owner.state.lock().unwrap().active = Some(Active {
            id: 1,
            epoch: 0,
            deadline: Instant::now() + DEADLINE,
            timed_out: false,
            cancelled: false,
            signal: Signal::new(),
            timer: Timer::new(),
            slot: Arc::downgrade(&slot),
            _permit: owner.claim_modal().unwrap(),
        });
        let mut request = DialogRequest {
            owner: Arc::clone(&owner),
            slot,
            id: 1,
            epoch: 0,
            deadline: Instant::now() + DEADLINE,
            delivered: false,
        };
        owner.complete(1, Ok(DialogOutcome::Acknowledged));
        owner.close_silent();
        assert!(matches!(
            Pin::new(&mut request).poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Err(DialogError::Closed))
        ));
    }
}
