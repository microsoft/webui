// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Trusted-host, per-window PNG clipboard admission and lifecycle.
//! No page global, system clipboard test access, or issue opening.

use std::future::Future;
use std::pin::Pin;
#[cfg(any(target_os = "macos", windows))]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(any(target_os = "macos", windows))]
use std::sync::{Arc, Condvar, Mutex, Weak};
#[cfg(any(target_os = "macos", windows))]
use std::task::Waker;
use std::task::{Context, Poll};
#[cfg(any(target_os = "macos", windows))]
use std::time::{Duration, Instant};

#[cfg(any(target_os = "macos", windows))]
use crate::capture::{CaptureState, CaptureToken};

#[cfg(any(target_os = "macos", windows))]
const CLIPBOARD_DEADLINE: Duration = Duration::from_secs(10);
#[cfg(any(target_os = "macos", windows))]
const MAX_CLIPBOARD_DEADLINE_THREADS_PER_WINDOW: usize = 8;

/// A host-owned native PNG clipboard write failure. Failed OS writes leave
/// the captured-content resource available for explicit retry.
#[non_exhaustive]
#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum ClipboardError {
    /// This platform has no native PNG clipboard adapter.
    #[error("native PNG clipboard writes are unsupported on this platform")]
    Unsupported,
    /// The capture no longer belongs to this window and document.
    #[error("captured PNG was released, replaced, or belongs to another window")]
    Released,
    /// Native window or verified host has retired.
    #[error("clipboard window or verified host has closed")]
    Closed,
    /// One native clipboard operation has not acknowledged completion.
    #[error("a native PNG clipboard write is already in progress")]
    Busy,
    /// Navigation, viewport change, or dropped request invalidated the write.
    #[error("clipboard write was cancelled by navigation or a new capture")]
    Cancelled,
    /// The native operation did not acknowledge within its bounded deadline.
    #[error("native PNG clipboard write did not complete within ten seconds")]
    Timeout,
    /// The native clipboard refused the PNG format or bytes.
    #[error("native clipboard rejected the PNG write")]
    Rejected,
    /// Another writer holds or changed the clipboard.
    #[error("the native clipboard is unavailable or changed during PNG verification")]
    Contended,
    /// Native readback failed or differed from the captured PNG.
    #[error("native PNG clipboard readback did not match captured bytes")]
    Readback,
    /// The captured resource lacks a valid bounded PNG representation.
    #[error("captured PNG is invalid or exceeds the native byte budget")]
    InvalidData,
    /// The native UI scheduler or timer could not admit this operation.
    #[error("native clipboard scheduler is unavailable")]
    Scheduler,
    /// Too many previous clipboard deadline threads have not exited yet.
    #[error("native clipboard deadline capacity is exhausted; retry after pending work settles")]
    Overloaded,
    /// A native clipboard or memory operation failed.
    #[error("native PNG clipboard {operation} failed with OS code {code}")]
    Os {
        /// Native operation which failed.
        operation: &'static str,
        /// Numeric OS error, never page-provided text.
        code: u32,
    },
}

/// Validate only the logical PNG bytes; HGLOBAL may reserve extra trailing
/// capacity which must not be read as part of the encoded resource.
#[cfg(any(windows, test))]
pub(crate) fn valid_png_bytes(png: &[u8]) -> bool {
    (36..=crate::MAX_WEB_CAPTURE_PNG_BYTES).contains(&png.len())
        && png.starts_with(b"\x89PNG\r\n\x1a\n")
        && png.ends_with(b"\0\0\0\0IEND\xaeB`\x82")
}

#[cfg(any(windows, test))]
pub(crate) fn matches_logical_png(expected: &[u8], observed: &[u8], allocation: usize) -> bool {
    valid_png_bytes(expected)
        && allocation >= expected.len()
        && observed.len() == expected.len()
        && expected == observed
}

#[cfg(any(target_os = "macos", windows))]
struct Slot(Mutex<SlotState>);

#[cfg(any(target_os = "macos", windows))]
struct SlotState {
    result: Option<Result<(), ClipboardError>>,
    waker: Option<Waker>,
}

#[cfg(any(target_os = "macos", windows))]
impl Slot {
    fn complete(&self, result: Result<(), ClipboardError>) {
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

    fn poll(&self, cx: &mut Context<'_>) -> Poll<Result<(), ClipboardError>> {
        let Ok(mut state) = self.0.lock() else {
            return Poll::Ready(Err(ClipboardError::Scheduler));
        };
        if let Some(result) = state.result.take() {
            return Poll::Ready(result);
        }
        state.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

#[cfg(any(target_os = "macos", windows))]
struct DeadlineTimer {
    finished: Mutex<bool>,
    changed: Condvar,
}

#[cfg(any(target_os = "macos", windows))]
impl DeadlineTimer {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            finished: Mutex::new(false),
            changed: Condvar::new(),
        })
    }

    fn finish(&self) {
        if let Ok(mut finished) = self.finished.lock() {
            *finished = true;
            self.changed.notify_one();
        }
    }
}

#[cfg(any(target_os = "macos", windows))]
struct Active {
    id: u64,
    token: CaptureToken,
    deadline: Instant,
    cancelled: bool,
    writing: bool,
    timed_out: bool,
    timer: Arc<DeadlineTimer>,
    slot: Weak<Slot>,
}

#[cfg(any(target_os = "macos", windows))]
struct TimerReservation(Arc<ClipboardState>);

#[cfg(any(target_os = "macos", windows))]
impl Drop for TimerReservation {
    fn drop(&mut self) {
        self.0.active_timer_threads.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(any(target_os = "macos", windows))]
struct State {
    closed: bool,
    next_id: u64,
    active: Option<Active>,
}

#[cfg(any(target_os = "macos", windows))]
pub(crate) struct ClipboardState {
    lifetime: crate::HostLifetime,
    capture: Arc<CaptureState>,
    state: Mutex<State>,
    #[cfg(target_os = "macos")]
    dispatch: Mutex<Option<Arc<crate::macos::clipboard::Dispatch>>>,
    #[cfg(windows)]
    target: AtomicUsize,
    active_timer_threads: AtomicUsize,
}

#[cfg(any(target_os = "macos", windows))]
impl ClipboardState {
    pub(crate) fn new(lifetime: crate::HostLifetime, capture: Arc<CaptureState>) -> Arc<Self> {
        Arc::new(Self {
            lifetime,
            capture,
            state: Mutex::new(State {
                closed: false,
                next_id: 0,
                active: None,
            }),
            #[cfg(target_os = "macos")]
            dispatch: Mutex::new(None),
            #[cfg(windows)]
            target: AtomicUsize::new(0),
            active_timer_threads: AtomicUsize::new(0),
        })
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn attach(&self, dispatch: Arc<crate::macos::clipboard::Dispatch>) {
        if let Ok(mut attached) = self.dispatch.lock() {
            *attached = Some(dispatch);
        }
    }

    #[cfg(windows)]
    pub(crate) fn attach_window(&self, hwnd: usize) {
        self.target.store(hwnd, Ordering::Release);
    }

    #[cfg(all(test, target_os = "macos"))]
    pub(crate) fn test_is_closed(&self) -> bool {
        self.state.lock().is_ok_and(|state| state.closed)
    }

    #[cfg(all(test, target_os = "macos"))]
    pub(crate) fn test_claim(&self, token: CaptureToken) -> Result<u64, ClipboardError> {
        let slot = Arc::new(Slot(Mutex::new(SlotState {
            result: None,
            waker: None,
        })));
        self.claim(
            token,
            &slot,
            Instant::now() + CLIPBOARD_DEADLINE,
            &DeadlineTimer::new(),
        )
    }

    pub(crate) fn begin(
        self: &Arc<Self>,
        content: &crate::CapturedContent,
    ) -> Result<ClipboardRequest, ClipboardError> {
        if !self.lifetime.is_active() {
            self.close_silent();
            return Err(ClipboardError::Closed);
        }
        let token = content.token();
        // Arc is cloned for validation only, not a full PNG copy or a queued
        // lease. The UI callback borrows a fresh Arc after revalidation.
        drop(self.capture.lease_png(token).map_err(map_capture_error)?);
        #[cfg(target_os = "macos")]
        let dispatch = self
            .dispatch
            .lock()
            .map_err(|_| ClipboardError::Scheduler)?
            .as_ref()
            .cloned()
            .ok_or(ClipboardError::Scheduler)?;
        let slot = Arc::new(Slot(Mutex::new(SlotState {
            result: None,
            waker: None,
        })));
        let deadline = Instant::now() + CLIPBOARD_DEADLINE;
        let timer = DeadlineTimer::new();
        let id = self.claim(token, &slot, deadline, &timer)?;
        let reservation = match self.reserve_timer() {
            Ok(reservation) => reservation,
            Err(error) => {
                self.abort_unsubmitted(id);
                return Err(error);
            }
        };
        if Self::start_deadline(self, &timer, id, deadline, reservation).is_err() {
            self.abort_unsubmitted(id);
            return Err(ClipboardError::Scheduler);
        }
        #[cfg(target_os = "macos")]
        let submitted = dispatch.submit(id);
        #[cfg(windows)]
        let submitted = crate::windows::clipboard::submit(
            Arc::clone(self),
            id,
            self.target.load(Ordering::Acquire),
        );
        if let Err(error) = submitted {
            self.abort_unsubmitted(id);
            return Err(error);
        }
        Ok(ClipboardRequest {
            owner: Arc::clone(self),
            slot,
            id,
            token,
            deadline,
            delivered: false,
        })
    }

    fn claim(
        &self,
        token: CaptureToken,
        slot: &Arc<Slot>,
        deadline: Instant,
        timer: &Arc<DeadlineTimer>,
    ) -> Result<u64, ClipboardError> {
        let mut state = self.state.lock().map_err(|_| ClipboardError::Scheduler)?;
        if state.closed || !self.lifetime.is_active() {
            return Err(ClipboardError::Closed);
        }
        if state.active.is_some() {
            return Err(ClipboardError::Busy);
        }
        let id = state
            .next_id
            .checked_add(1)
            .ok_or(ClipboardError::Scheduler)?;
        state.next_id = id;
        state.active = Some(Active {
            id,
            token,
            deadline,
            cancelled: false,
            writing: false,
            timed_out: false,
            timer: Arc::clone(timer),
            slot: Arc::downgrade(slot),
        });
        Ok(id)
    }

    fn reserve_timer(self: &Arc<Self>) -> Result<TimerReservation, ClipboardError> {
        self.active_timer_threads
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < MAX_CLIPBOARD_DEADLINE_THREADS_PER_WINDOW).then_some(count + 1)
            })
            .map_err(|_| ClipboardError::Overloaded)?;
        Ok(TimerReservation(Arc::clone(self)))
    }

    fn start_deadline(
        owner: &Arc<Self>,
        timer: &Arc<DeadlineTimer>,
        id: u64,
        deadline: Instant,
        reservation: TimerReservation,
    ) -> Result<(), ClipboardError> {
        let timer = Arc::clone(timer);
        let weak_owner = Arc::downgrade(owner);
        std::thread::Builder::new()
            .name("webui-clipboard-deadline".into())
            .spawn(move || {
                let _reservation = reservation;
                let Ok(finished) = timer.finished.lock() else {
                    return;
                };
                let remaining = deadline.saturating_duration_since(Instant::now());
                let Ok((finished, _)) =
                    timer
                        .changed
                        .wait_timeout_while(finished, remaining, |done| !*done)
                else {
                    return;
                };
                if !*finished {
                    drop(finished);
                    if let Some(owner) = weak_owner.upgrade() {
                        owner.timeout(id);
                    }
                }
            })
            .map(|_| ())
            .map_err(|_| ClipboardError::Scheduler)
    }

    fn abort_unsubmitted(&self, id: u64) {
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
            active.slot.upgrade()
        } else {
            None
        };
        if let Some(slot) = slot {
            slot.complete(Err(ClipboardError::Timeout));
        }
        // Preserve Busy until the native operation actually returns.
    }

    #[allow(clippy::rc_buffer)]
    pub(crate) fn payload(&self, id: u64) -> Result<Arc<Vec<u8>>, ClipboardError> {
        if !self.lifetime.is_active() {
            return Err(ClipboardError::Closed);
        }
        let token = {
            let state = self.state.lock().map_err(|_| ClipboardError::Scheduler)?;
            let active = state
                .active
                .as_ref()
                .filter(|active| active.id == id)
                .ok_or(ClipboardError::Cancelled)?;
            if state.closed || active.cancelled {
                return Err(ClipboardError::Cancelled);
            }
            if active.timed_out || Instant::now() >= active.deadline {
                return Err(ClipboardError::Timeout);
            }
            active.token
        };
        self.capture.lease_png(token).map_err(map_capture_error)
    }

    /// Admission boundary immediately before native clipboard mutation.
    /// Called after preparing bounded bytes and before replacing contents.
    /// Cancelling a queued request wins this lock; writing cannot be retracted.
    pub(crate) fn begin_write(&self, id: u64) -> Result<(), ClipboardError> {
        let mut state = self.state.lock().map_err(|_| ClipboardError::Scheduler)?;
        if state.closed || !self.lifetime.is_active() {
            return Err(ClipboardError::Closed);
        }
        let active = state
            .active
            .as_mut()
            .filter(|active| active.id == id)
            .ok_or(ClipboardError::Cancelled)?;
        if active.cancelled {
            return Err(ClipboardError::Cancelled);
        }
        if active.timed_out || Instant::now() >= active.deadline {
            return Err(ClipboardError::Timeout);
        }
        if active.writing {
            return Err(ClipboardError::Busy);
        }
        // This nested, non-blocking admission update is atomic with respect
        // to capture release/retake: cancellation before transition wins.
        // Both locks are released before the subsequent native mutation.
        self.capture
            .with_valid_token(active.token, || active.writing = true)
            .map_err(map_capture_error)?;
        Ok(())
    }

    pub(crate) fn complete(&self, id: u64, result: Result<(), ClipboardError>) {
        let active = if let Ok(mut state) = self.state.lock() {
            if state.active.as_ref().is_none_or(|active| active.id != id) {
                return;
            }
            let active = state.active.take();
            let closed = state.closed || !self.lifetime.is_active();
            (active, closed)
        } else {
            return;
        };
        let (Some(active), closed) = active else {
            return;
        };
        active.timer.finish();
        let answer = if closed {
            Err(ClipboardError::Closed)
        } else if active.cancelled {
            Err(ClipboardError::Cancelled)
        } else if active.timed_out || Instant::now() >= active.deadline {
            Err(ClipboardError::Timeout)
        } else {
            self.capture
                .lease_png(active.token)
                .map_err(map_capture_error)
                .and(result)
        };
        if let Some(slot) = active.slot.upgrade() {
            slot.complete(answer);
        }
    }

    pub(crate) fn cancel_invalidated(&self) {
        let id = self.state.lock().ok().and_then(|state| {
            state
                .active
                .as_ref()
                .map(|active| (active.id, active.token))
        });
        if let Some((id, token)) = id {
            if self.capture.lease_png(token).is_err() {
                self.cancel(id, ClipboardError::Released);
            }
        }
    }

    pub(crate) fn cancel_token(&self, token: CaptureToken) {
        let id = self.state.lock().ok().and_then(|state| {
            state
                .active
                .as_ref()
                .filter(|active| active.token == token)
                .map(|active| active.id)
        });
        if let Some(id) = id {
            self.cancel(id, ClipboardError::Released);
        }
    }

    fn cancel(&self, id: u64, reason: ClipboardError) {
        let slot = if let Ok(mut state) = self.state.lock() {
            let Some(active) = state.active.as_mut().filter(|active| active.id == id) else {
                return;
            };
            active.cancelled = true;
            active.timer.finish();
            active.slot.upgrade()
        } else {
            None
        };
        if let Some(slot) = slot {
            slot.complete(Err(reason));
        }
    }

    pub(crate) fn navigation_changed(&self) {
        let id = self
            .state
            .lock()
            .ok()
            .and_then(|state| state.active.as_ref().map(|active| active.id));
        if let Some(id) = id {
            self.cancel(id, ClipboardError::Cancelled);
        }
    }

    pub(crate) fn close_silent(&self) {
        // Also called under HostLifetime's callback lock. No host Future
        // waker or OS clipboard call may be invoked in this synchronous path.
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
            if let Some(active) = state.active.as_mut() {
                active.cancelled = true;
            }
        }
    }

    pub(crate) fn notify_closed(&self) {
        self.close_silent();
        let slot = if let Ok(mut state) = self.state.lock() {
            state.active.as_mut().and_then(|active| {
                active.timer.finish();
                active.slot.upgrade()
            })
        } else {
            None
        };
        if let Some(slot) = slot {
            slot.complete(Err(ClipboardError::Closed));
        }
    }

    fn abandon(&self, id: u64) {
        if let Ok(mut state) = self.state.lock() {
            if let Some(active) = state.active.as_mut().filter(|active| active.id == id) {
                active.cancelled = true;
                active.timer.finish();
            }
        }
    }
}

#[cfg(any(target_os = "macos", windows))]
fn map_capture_error(error: crate::CaptureError) -> ClipboardError {
    match error {
        crate::CaptureError::Closed => ClipboardError::Closed,
        crate::CaptureError::Released => ClipboardError::Released,
        _ => ClipboardError::Released,
    }
}

/// Awaitable native clipboard acknowledgement. A successful result means
/// The OS accepted the registered PNG format and immediate readback matched.
#[must_use = "await OS write and verified PNG readback before any host URL opener"]
pub struct ClipboardRequest {
    #[cfg(any(target_os = "macos", windows))]
    owner: Arc<ClipboardState>,
    #[cfg(any(target_os = "macos", windows))]
    slot: Arc<Slot>,
    #[cfg(any(target_os = "macos", windows))]
    id: u64,
    #[cfg(any(target_os = "macos", windows))]
    token: CaptureToken,
    #[cfg(any(target_os = "macos", windows))]
    deadline: Instant,
    #[cfg(any(target_os = "macos", windows))]
    delivered: bool,
}

impl Future for ClipboardRequest {
    type Output = Result<(), ClipboardError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        #[cfg(any(target_os = "macos", windows))]
        {
            let this = self.get_mut();
            if !this.owner.lifetime.is_active() {
                this.owner.close_silent();
                return Poll::Ready(Err(ClipboardError::Closed));
            }
            if Instant::now() >= this.deadline {
                this.owner.timeout(this.id);
            }
            let result = this.slot.poll(cx);
            if result.is_ready() {
                this.delivered = true;
            }
            match result {
                Poll::Ready(Ok(())) => Poll::Ready(
                    this.owner
                        .capture
                        .lease_png(this.token)
                        .map(|_| ())
                        .map_err(map_capture_error),
                ),
                other => other,
            }
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = (self, cx);
            Poll::Ready(Err(ClipboardError::Unsupported))
        }
    }
}

impl Drop for ClipboardRequest {
    fn drop(&mut self) {
        #[cfg(any(target_os = "macos", windows))]
        if !self.delivered {
            self.owner.abandon(self.id);
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::task::Waker;

    static NEXT_GENERATION: AtomicU64 = AtomicU64::new(41);

    #[test]
    fn png_readback_compares_only_bounded_logical_bytes_not_global_padding() {
        let png = crate::macos::clipboard::TEST_PNG;
        assert!(valid_png_bytes(&png));
        assert!(matches_logical_png(&png, &png, png.len() + 15));
        assert!(!matches_logical_png(&png, &png, png.len() - 1));
        let mut corrupt = png;
        corrupt[30] ^= 1;
        assert!(!matches_logical_png(&png, &corrupt, png.len() + 15));
        assert!(!valid_png_bytes(&[0; 36]));
        let too_large = vec![0; crate::MAX_WEB_CAPTURE_PNG_BYTES + 1];
        assert!(!valid_png_bytes(&too_large));
    }

    #[test]
    fn irreversible_write_gate_rejects_released_retaken_and_cancelled_tokens() {
        let (capture, clipboard, _host, content) = setup();
        let initial = pending(&clipboard, content.token());
        assert_eq!(clipboard.begin_write(initial.id), Ok(()));
        clipboard.complete(initial.id, Err(ClipboardError::Rejected));
        let next = pending(&clipboard, content.token());
        capture.release(&content).unwrap();
        assert_eq!(
            clipboard.begin_write(next.id),
            Err(ClipboardError::Released)
        );
        clipboard.complete(next.id, Err(ClipboardError::Released));
        let (capture, clipboard, _host, current) = setup();
        let resized = pending(&clipboard, current.token());
        capture.viewport_changed();
        assert_eq!(
            clipboard.begin_write(resized.id),
            Err(ClipboardError::Released)
        );
        clipboard.complete(resized.id, Err(ClipboardError::Released));
        let (_capture, clipboard, _host, current) = setup();
        let cancelled = pending(&clipboard, current.token());
        clipboard.navigation_changed();
        assert_eq!(
            clipboard.begin_write(cancelled.id),
            Err(ClipboardError::Cancelled)
        );
    }

    fn setup() -> (
        Arc<CaptureState>,
        Arc<ClipboardState>,
        crate::HostLifetimeOwner,
        crate::CapturedContent,
    ) {
        let (host, lifetime) = crate::HostLifetime::new();
        let capture = CaptureState::new(
            lifetime.clone(),
            NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
        );
        capture.test_store_retained(25_000);
        let content = capture.test_content().unwrap();
        let clipboard = ClipboardState::new(lifetime, Arc::clone(&capture));
        (capture, clipboard, host, content)
    }

    fn pending(owner: &Arc<ClipboardState>, token: CaptureToken) -> ClipboardRequest {
        let slot = Arc::new(Slot(Mutex::new(SlotState {
            result: None,
            waker: None,
        })));
        let deadline = Instant::now() + CLIPBOARD_DEADLINE;
        let timer = DeadlineTimer::new();
        let id = owner.claim(token, &slot, deadline, &timer).unwrap();
        ClipboardRequest {
            owner: Arc::clone(owner),
            slot,
            id,
            token,
            deadline,
            delivered: false,
        }
    }

    #[test]
    fn foreign_released_busy_and_os_failure_keep_retry_resource() {
        let (capture, clipboard, _host, content) = setup();
        let (_, foreign, _foreign_host, _) = setup();
        let first_lease = capture.lease_png(content.token()).unwrap();
        let second_lease = capture.lease_png(content.token()).unwrap();
        assert!(Arc::ptr_eq(&first_lease, &second_lease));
        assert!(matches!(
            foreign.begin(&content),
            Err(ClipboardError::Released)
        ));
        let mut request = pending(&clipboard, content.token());
        let another = Arc::new(Slot(Mutex::new(SlotState {
            result: None,
            waker: None,
        })));
        assert!(matches!(
            clipboard.claim(
                content.token(),
                &another,
                Instant::now() + CLIPBOARD_DEADLINE,
                &DeadlineTimer::new(),
            ),
            Err(ClipboardError::Busy)
        ));
        clipboard.complete(request.id, Err(ClipboardError::Rejected));
        assert!(matches!(
            Pin::new(&mut request).poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Err(ClipboardError::Rejected))
        ));
        assert_eq!(capture.lease_png(content.token()).unwrap().len(), 25_000);
        let mut retry = pending(&clipboard, content.token());
        clipboard.complete(retry.id, Ok(()));
        assert!(matches!(
            Pin::new(&mut retry).poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(()))
        ));
        capture.release(&content).unwrap();
        assert!(matches!(
            clipboard.begin(&content),
            Err(ClipboardError::Released)
        ));
    }

    #[test]
    fn public_host_accessor_rejects_foreign_capture_without_native_attachment() {
        let (_capture, _clipboard, _host, content) = setup();
        let events = crate::EventRegistry::default();
        let (_other_host, lifetime) = crate::HostLifetime::new();
        let services = crate::NativeServices::new(&events, Arc::default(), lifetime).unwrap();
        assert!(matches!(
            services.write_capture_to_clipboard(&content),
            Err(ClipboardError::Released)
        ));
    }

    #[test]
    fn release_navigation_close_and_late_completion_fail_closed() {
        let (capture, clipboard, host, content) = setup();
        let mut released = pending(&clipboard, content.token());
        capture.release(&content).unwrap();
        clipboard.cancel_invalidated();
        assert!(matches!(
            Pin::new(&mut released).poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Err(ClipboardError::Released))
        ));
        assert!(clipboard.state.lock().unwrap().active.is_some());
        clipboard.complete(released.id, Ok(()));
        assert!(clipboard.state.lock().unwrap().active.is_none());

        capture.test_store_retained(25_000);
        let current = capture.test_content().unwrap();
        let mut navigating = pending(&clipboard, current.token());
        capture.invalidate(2, false);
        clipboard.navigation_changed();
        assert!(matches!(
            Pin::new(&mut navigating).poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Err(ClipboardError::Cancelled))
        ));
        assert!(matches!(
            clipboard.payload(navigating.id),
            Err(ClipboardError::Cancelled)
        ));
        clipboard.complete(navigating.id, Ok(()));
        clipboard.close_silent();
        assert!(clipboard.test_is_closed());
        host.revoke().unwrap();
        assert!(matches!(
            clipboard.begin(&current),
            Err(ClipboardError::Closed)
        ));
    }

    #[test]
    fn timer_expires_future_but_keeps_native_busy_until_callback() {
        let (_capture, clipboard, _host, content) = setup();
        let mut request = pending(&clipboard, content.token());
        let deadline = Instant::now() + Duration::from_millis(25);
        let timer = {
            let mut state = clipboard.state.lock().unwrap();
            let active = state.active.as_mut().unwrap();
            active.deadline = deadline;
            Arc::clone(&active.timer)
        };
        request.deadline = deadline;
        let reservation = clipboard.reserve_timer().unwrap();
        ClipboardState::start_deadline(&clipboard, &timer, request.id, deadline, reservation)
            .unwrap();
        let until = Instant::now() + Duration::from_secs(2);
        while !clipboard
            .state
            .lock()
            .unwrap()
            .active
            .as_ref()
            .is_some_and(|active| active.timed_out)
        {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(matches!(
            Pin::new(&mut request).poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Err(ClipboardError::Timeout))
        ));
        assert!(clipboard.state.lock().unwrap().active.is_some());
        assert!(matches!(
            clipboard.payload(request.id),
            Err(ClipboardError::Timeout)
        ));
        clipboard.complete(request.id, Ok(()));
        assert!(clipboard.state.lock().unwrap().active.is_none());
    }

    #[test]
    fn deadline_reservations_are_capped_and_rapid_cleanup_does_not_accumulate() {
        let (_capture, clipboard, _host, content) = setup();
        let permits = (0..MAX_CLIPBOARD_DEADLINE_THREADS_PER_WINDOW)
            .map(|_| clipboard.reserve_timer().unwrap())
            .collect::<Vec<_>>();
        assert!(matches!(
            clipboard.reserve_timer(),
            Err(ClipboardError::Overloaded)
        ));
        assert_eq!(
            clipboard.active_timer_threads.load(Ordering::Acquire),
            MAX_CLIPBOARD_DEADLINE_THREADS_PER_WINDOW
        );
        drop(permits);
        for _ in 0..100 {
            let permit = clipboard.reserve_timer().unwrap();
            drop(permit);
            assert_eq!(clipboard.active_timer_threads.load(Ordering::Acquire), 0);
        }
        for _ in 0..100 {
            let mut operation = pending(&clipboard, content.token());
            let timer = clipboard
                .state
                .lock()
                .unwrap()
                .active
                .as_ref()
                .unwrap()
                .timer
                .clone();
            let until = Instant::now() + Duration::from_secs(5);
            let permit = loop {
                match clipboard.reserve_timer() {
                    Ok(permit) => break permit,
                    Err(ClipboardError::Overloaded) if Instant::now() < until => {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("deadline capacity did not recover: {error:?}"),
                }
            };
            ClipboardState::start_deadline(
                &clipboard,
                &timer,
                operation.id,
                operation.deadline,
                permit,
            )
            .unwrap();
            clipboard.complete(operation.id, Err(ClipboardError::Rejected));
            assert!(matches!(
                Pin::new(&mut operation).poll(&mut Context::from_waker(Waker::noop())),
                Poll::Ready(Err(ClipboardError::Rejected))
            ));
            assert!(
                clipboard.active_timer_threads.load(Ordering::Acquire)
                    <= MAX_CLIPBOARD_DEADLINE_THREADS_PER_WINDOW
            );
        }
        let until = Instant::now() + Duration::from_secs(5);
        while clipboard.active_timer_threads.load(Ordering::Acquire) != 0 {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
