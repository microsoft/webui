// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! One bounded, main-queue appearance request per native window.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use objc2::rc::Weak as ObjcWeak;
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSWindow,
};
use objc2_web_kit::WKWebView;

use super::{ThemeMode, ThemeState};
use crate::{NativeServiceError, ThemeRequest};

const DEADLINE: Duration = Duration::from_secs(10);
const MAX_DEADLINE_THREADS: usize = 8;
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static TARGETS: RefCell<HashMap<u64, Rc<Target>>> = RefCell::new(HashMap::new());
}

#[link(name = "System")]
unsafe extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_async_f(
        queue: *mut c_void,
        context: *mut c_void,
        work: unsafe extern "C" fn(*mut c_void),
    );
}

struct Target {
    window: ObjcWeak<NSWindow>,
    view: ObjcWeak<WKWebView>,
    controller: Weak<Controller>,
}

struct Pending {
    mode: ThemeMode,
    epoch: u64,
    serial: u64,
    slot: Weak<Slot>,
}

struct State {
    snapshot: Option<ThemeState>,
    pending: Option<Pending>,
    // Cancellation retires the request, not its native ticket. Admission
    // stays reserved through setup and drain, even when pending is empty.
    busy: bool,
    target: Option<u64>,
    epoch: u64,
    serial: u64,
    closed: bool,
}

pub(crate) struct Controller {
    state: Mutex<State>,
    events: crate::EventRegistry,
    // Weak host authority: queue admission is not permission to mutate a
    // window after its owning listener/connection has been retired.
    lifetime: crate::HostLifetime,
    active_deadlines: AtomicUsize,
}

struct DeadlineReservation(Arc<Controller>);
impl Drop for DeadlineReservation {
    fn drop(&mut self) {
        self.0.active_deadlines.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Controller {
    pub(crate) fn new(events: crate::EventRegistry, lifetime: crate::HostLifetime) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                snapshot: None,
                pending: None,
                busy: false,
                target: None,
                epoch: 0,
                serial: 0,
                closed: false,
            }),
            events,
            lifetime,
            active_deadlines: AtomicUsize::new(0),
        })
    }

    fn require_active(&self) -> Result<(), NativeServiceError> {
        if self.lifetime.is_active() {
            Ok(())
        } else {
            Err(NativeServiceError::Closed)
        }
    }

    /// Called only on the AppKit thread, after attaching the webview.
    pub(crate) fn attach(self: &Arc<Self>, window: &NSWindow, view: &WKWebView) -> Registration {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        TARGETS.with(|targets| {
            targets.borrow_mut().insert(
                id,
                Rc::new(Target {
                    window: ObjcWeak::new(window),
                    view: ObjcWeak::new(view),
                    controller: Arc::downgrade(self),
                }),
            );
        });
        if let Ok(mut state) = self.state.lock() {
            state.target = Some(id);
            state.snapshot = Some(ThemeState {
                mode: ThemeMode::System,
                dark: super::super::macos::theme::is_dark_view(view),
            });
        }
        Registration {
            id,
            controller: Arc::clone(self),
            _ui_only: std::marker::PhantomData,
        }
    }

    pub(crate) fn snapshot(&self) -> Result<ThemeState, NativeServiceError> {
        self.require_active()?;
        let state = self
            .state
            .lock()
            .map_err(|_| NativeServiceError::Unavailable)?;
        if state.closed {
            return Err(NativeServiceError::Closed);
        }
        let snapshot = state.snapshot.ok_or(NativeServiceError::ThemeUnavailable)?;
        self.require_active()?;
        Ok(snapshot)
    }

    pub(crate) fn observe(&self, dark: bool) -> bool {
        if self.require_active().is_err() {
            return false;
        }
        if let Ok(mut state) = self.state.lock() {
            if state.closed || !self.lifetime.is_active() {
                return false;
            }
            if let Some(snapshot) = state.snapshot.as_mut() {
                snapshot.dark = dark;
            }
            return true;
        }
        false
    }

    pub(crate) fn set(
        self: &Arc<Self>,
        mode: ThemeMode,
    ) -> Result<ThemeRequest, NativeServiceError> {
        self.set_with(mode, start_deadline, enqueue)
    }

    fn set_with(
        self: &Arc<Self>,
        mode: ThemeMode,
        start: impl FnOnce(Arc<Slot>, DeadlineReservation) -> Result<(), NativeServiceError>,
        enqueue: impl FnOnce(u64, u64),
    ) -> Result<ThemeRequest, NativeServiceError> {
        let (id, serial, slot) = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| NativeServiceError::Unavailable)?;
            if state.closed || !self.lifetime.is_active() {
                return Err(NativeServiceError::Closed);
            }
            let id = state.target.ok_or(NativeServiceError::ThemeUnavailable)?;
            if state.busy {
                return Err(NativeServiceError::ThemeBusy);
            }
            let slot = Arc::new(Slot::new());
            let epoch = state.epoch;
            state.serial = state.serial.wrapping_add(1);
            let serial = state.serial;
            state.pending = Some(Pending {
                mode,
                epoch,
                serial,
                slot: Arc::downgrade(&slot),
            });
            state.busy = true;
            (id, serial, slot)
        };
        if self
            .active_deadlines
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < MAX_DEADLINE_THREADS).then_some(count + 1)
            })
            .is_err()
        {
            self.cancel_claim(serial, NativeServiceError::Overloaded);
            return Err(NativeServiceError::Overloaded);
        }
        let reservation = DeadlineReservation(Arc::clone(self));
        if start(Arc::clone(&slot), reservation).is_err() {
            self.cancel_claim(serial, NativeServiceError::Unavailable);
            return Err(NativeServiceError::Unavailable);
        }
        enqueue(id, serial);
        Ok(ThemeRequest {
            inner: Request {
                controller: Arc::clone(self),
                slot,
            },
        })
    }

    pub(crate) fn cancel(&self, error: NativeServiceError, close: bool) {
        let pending = if let Ok(mut state) = self.state.lock() {
            state.epoch = state.epoch.wrapping_add(1);
            state.closed |= close;
            state.pending.take()
        } else {
            None
        };
        if let Some(slot) = pending.and_then(|pending| pending.slot.upgrade()) {
            slot.complete(Err(error));
        }
    }

    fn cancel_claim(&self, serial: u64, error: NativeServiceError) {
        let slot = if let Ok(mut state) = self.state.lock() {
            if state.serial == serial {
                state.busy = false;
                state
                    .pending
                    .take()
                    .and_then(|pending| pending.slot.upgrade())
            } else {
                None
            }
        } else {
            None
        };
        if let Some(slot) = slot {
            slot.complete(Err(error));
        }
    }

    fn apply(&self, target: &Target, serial: u64) {
        self.apply_with(serial, |pending| self.apply_native(target, pending));
    }

    fn apply_with(
        &self,
        serial: u64,
        apply: impl FnOnce(&Pending) -> Result<ThemeState, NativeServiceError>,
    ) {
        let pending = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            if !state.closed
                && state
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.serial == serial)
            {
                state.pending.take()
            } else {
                None
            }
        };
        let Some(pending) = pending else {
            self.release(serial);
            return;
        };
        let Some(slot) = pending.slot.upgrade() else {
            self.release(serial);
            return;
        };
        if slot.is_finished() {
            self.release(serial);
            return;
        }
        let result = self.require_active().and_then(|()| apply(&pending));
        self.release(serial);
        slot.complete(result);
    }

    fn apply_native(
        &self,
        target: &Target,
        pending: &Pending,
    ) -> Result<ThemeState, NativeServiceError> {
        let window = target.window.load().ok_or(NativeServiceError::Closed)?;
        let view = target.view.load().ok_or(NativeServiceError::Closed)?;
        if !view
            .window()
            .is_some_and(|attached| std::ptr::eq(&*attached, &*window))
        {
            return Err(NativeServiceError::ThemeUnavailable);
        }
        // SAFETY: AppKit's named appearances are static platform constants.
        let appearance = match pending.mode {
            ThemeMode::Light => NSAppearance::appearanceNamed(unsafe { NSAppearanceNameAqua }),
            ThemeMode::Dark => NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua }),
            ThemeMode::System => None,
        };
        if pending.mode != ThemeMode::System && appearance.is_none() {
            return Err(NativeServiceError::ThemeUnavailable);
        }
        // A queued request may outlive HostLifetimeOwner::revoke() while
        // AppKit has not yet processed its independent window-close wake.
        self.require_active()?;
        window.setAppearance(appearance.as_deref());
        self.require_active()?;
        let snapshot = ThemeState {
            mode: pending.mode,
            dark: super::super::macos::theme::is_dark_view(&view),
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| NativeServiceError::Unavailable)?;
        if state.closed || !self.lifetime.is_active() {
            return Err(NativeServiceError::Closed);
        }
        if state.epoch != pending.epoch {
            return Err(NativeServiceError::Cancelled);
        }
        state.snapshot = Some(snapshot);
        drop(state);
        self.require_active()?;
        crate::macos::dispatch_event(
            &self.events,
            &view,
            crate::DesktopEvent::ThemeChanged {
                dark: snapshot.dark,
            },
        );
        self.require_active()?;
        if self
            .state
            .lock()
            .map_err(|_| NativeServiceError::Unavailable)?
            .epoch
            != pending.epoch
        {
            return Err(NativeServiceError::Cancelled);
        }
        Ok(snapshot)
    }

    fn release(&self, serial: u64) {
        if let Ok(mut state) = self.state.lock() {
            if state.serial == serial && state.pending.is_none() {
                state.busy = false;
            }
        }
    }
}

fn start_deadline(
    timer: Arc<Slot>,
    reservation: DeadlineReservation,
) -> Result<(), NativeServiceError> {
    std::thread::Builder::new()
        .name("webui-theme-deadline".into())
        .spawn(move || {
            let _reservation = reservation;
            timer.wait_deadline();
        })
        .map(|_| ())
        .map_err(|_| NativeServiceError::Unavailable)
}

fn enqueue(id: u64, serial: u64) {
    let context = Box::into_raw(Box::new((id, serial))).cast::<c_void>();
    // SAFETY: GCD owns the ticket and drains on AppKit's main queue, the
    // only queue allowed to access the registered Objective-C weak references.
    unsafe {
        dispatch_async_f(
            std::ptr::addr_of!(_dispatch_main_q).cast_mut(),
            context,
            drain,
        );
    }
}

pub(crate) struct Registration {
    id: u64,
    controller: Arc<Controller>,
    _ui_only: std::marker::PhantomData<Rc<()>>,
}

impl Registration {
    pub(crate) fn close(&self) {
        self.controller.cancel(NativeServiceError::Closed, true);
        TARGETS.with(|targets| {
            targets.borrow_mut().remove(&self.id);
        });
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.close();
    }
}

unsafe extern "C" fn drain(context: *mut c_void) {
    // SAFETY: set() transfers exactly one boxed ID to this main-queue callback.
    let ticket = unsafe { Box::from_raw(context.cast::<(u64, u64)>()) };
    let target = TARGETS.with(|targets| targets.borrow().get(&ticket.0).cloned());
    if let Some(target) = target {
        if let Some(controller) = target.controller.upgrade() {
            controller.apply(&target, ticket.1);
        }
    }
}

struct SlotState {
    result: Option<Result<ThemeState, NativeServiceError>>,
    waker: Option<Waker>,
    finished: bool,
}

struct Slot {
    state: Mutex<SlotState>,
    changed: Condvar,
}

impl Slot {
    fn new() -> Self {
        Self {
            state: Mutex::new(SlotState {
                result: None,
                waker: None,
                finished: false,
            }),
            changed: Condvar::new(),
        }
    }
    fn is_finished(&self) -> bool {
        self.state.lock().is_ok_and(|state| state.finished)
    }
    fn complete(&self, result: Result<ThemeState, NativeServiceError>) {
        let wake = if let Ok(mut state) = self.state.lock() {
            if state.finished {
                return;
            }
            state.finished = true;
            state.result = Some(result);
            self.changed.notify_one();
            state.waker.take()
        } else {
            None
        };
        if let Some(wake) = wake {
            wake.wake();
        }
    }
    fn wait_deadline(&self) {
        let Ok(state) = self.state.lock() else { return };
        let Ok((state, _)) = self
            .changed
            .wait_timeout_while(state, DEADLINE, |state| !state.finished)
        else {
            return;
        };
        if !state.finished {
            drop(state);
            self.complete(Err(NativeServiceError::ThemeTimeout));
        }
    }
}

pub(crate) struct Request {
    controller: Arc<Controller>,
    slot: Arc<Slot>,
}

impl Future for Request {
    type Output = Result<ThemeState, NativeServiceError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if !self.controller.lifetime.is_active() {
            return Poll::Ready(Err(NativeServiceError::Closed));
        }
        let Ok(mut state) = self.slot.state.lock() else {
            return Poll::Ready(Err(NativeServiceError::Unavailable));
        };
        if let Some(result) = state.result.take() {
            drop(state);
            return Poll::Ready(if self.controller.lifetime.is_active() {
                result
            } else {
                Err(NativeServiceError::Closed)
            });
        }
        state.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        self.slot.complete(Err(NativeServiceError::Cancelled));
        if let Ok(mut state) = self.controller.state.lock() {
            if state.pending.as_ref().is_some_and(|pending| {
                pending
                    .slot
                    .upgrade()
                    .is_some_and(|slot| Arc::ptr_eq(&slot, &self.slot))
            }) {
                state.pending.take();
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::VecDeque;

    struct HeldDispatch {
        controller: Arc<Controller>,
        tickets: RefCell<VecDeque<(u64, u64)>>,
        deadlines: RefCell<Vec<(Arc<Slot>, DeadlineReservation)>>,
        applied: Cell<usize>,
    }

    impl HeldDispatch {
        fn new() -> (crate::HostLifetimeOwner, Self) {
            let (owner, lifetime) = crate::HostLifetime::new();
            let controller = Controller::new(crate::EventRegistry::default(), lifetime);
            controller.state.lock().unwrap().target = Some(1);
            (
                owner,
                Self {
                    controller,
                    tickets: RefCell::new(VecDeque::new()),
                    deadlines: RefCell::new(Vec::new()),
                    applied: Cell::new(0),
                },
            )
        }

        fn retire_deadlines(&self) {
            self.deadlines
                .borrow_mut()
                .retain(|(slot, _)| !slot.is_finished());
        }

        fn set(&self) -> Result<ThemeRequest, NativeServiceError> {
            self.retire_deadlines();
            self.controller.set_with(
                ThemeMode::Dark,
                |slot, reservation| {
                    self.deadlines.borrow_mut().push((slot, reservation));
                    Ok(())
                },
                |id, serial| self.tickets.borrow_mut().push_back((id, serial)),
            )
        }

        fn drain(&self) {
            self.drain_with(|pending| {
                self.applied.set(self.applied.get() + 1);
                Ok(ThemeState {
                    mode: pending.mode,
                    dark: true,
                })
            });
        }

        fn drain_with(
            &self,
            apply: impl FnOnce(&Pending) -> Result<ThemeState, NativeServiceError>,
        ) {
            let (_, serial) = self.tickets.borrow_mut().pop_front().unwrap();
            self.controller.apply_with(serial, apply);
            self.retire_deadlines();
        }
    }

    fn poll(request: &mut ThemeRequest) -> Poll<Result<ThemeState, NativeServiceError>> {
        Pin::new(request).poll(&mut Context::from_waker(Waker::noop()))
    }

    #[test]
    fn held_native_dispatch_bounds_ten_thousand_set_drop_attempts() {
        let (_owner, held) = HeldDispatch::new();
        for _ in 0..10_000 {
            match held.set() {
                Ok(request) => drop(request),
                Err(NativeServiceError::ThemeBusy) => {}
                Err(error) => panic!("unexpected admission error: {error}"),
            }
        }
        held.retire_deadlines();
        assert_eq!(held.tickets.borrow().len(), 1);
        assert_eq!(held.controller.active_deadlines.load(Ordering::Acquire), 0);
        held.drain();
        assert_eq!(held.applied.get(), 0);
        let mut retry = held.set().unwrap();
        held.drain();
        assert_eq!(held.applied.get(), 1);
        assert!(matches!(poll(&mut retry), Poll::Ready(Ok(_))));
        assert!(held.tickets.borrow().is_empty());
    }

    #[test]
    fn navigation_cannot_accumulate_cancelled_native_tickets() {
        let (_owner, held) = HeldDispatch::new();
        for _ in 0..10_000 {
            match held.set() {
                Ok(mut request) => {
                    held.controller.cancel(NativeServiceError::Cancelled, false);
                    assert!(matches!(
                        poll(&mut request),
                        Poll::Ready(Err(NativeServiceError::Cancelled))
                    ));
                }
                Err(NativeServiceError::ThemeBusy) => {
                    held.controller.cancel(NativeServiceError::Cancelled, false);
                }
                Err(error) => panic!("unexpected admission error: {error}"),
            }
        }
        assert_eq!(held.tickets.borrow().len(), 1);
        held.drain();
        assert_eq!(held.applied.get(), 0);
        let mut retry = held.set().unwrap();
        held.drain();
        assert!(matches!(poll(&mut retry), Poll::Ready(Ok(_))));
    }

    #[test]
    fn expired_request_keeps_admission_until_drain_with_or_without_future_drop() {
        for drop_before_drain in [false, true] {
            let (_owner, held) = HeldDispatch::new();
            let mut request = held.set().unwrap();
            request
                .inner
                .slot
                .complete(Err(NativeServiceError::ThemeTimeout));
            held.retire_deadlines();
            assert!(matches!(
                poll(&mut request),
                Poll::Ready(Err(NativeServiceError::ThemeTimeout))
            ));
            let request = if drop_before_drain {
                drop(request);
                None
            } else {
                Some(request)
            };
            assert_eq!(held.controller.active_deadlines.load(Ordering::Acquire), 0);
            assert!(matches!(held.set(), Err(NativeServiceError::ThemeBusy)));
            held.drain();
            assert_eq!(held.applied.get(), 0);
            let mut retry = held.set().unwrap();
            drop(request);
            assert!(matches!(held.set(), Err(NativeServiceError::ThemeBusy)));
            held.drain();
            assert!(matches!(poll(&mut retry), Poll::Ready(Ok(_))));
        }
    }

    #[test]
    fn cancellation_during_successful_setup_keeps_the_later_ticket_reserved() {
        for close in [false, true] {
            let (_owner, held) = HeldDispatch::new();
            let mut request = held
                .controller
                .set_with(
                    ThemeMode::Dark,
                    |slot, reservation| {
                        held.controller.cancel(
                            if close {
                                NativeServiceError::Closed
                            } else {
                                NativeServiceError::Cancelled
                            },
                            close,
                        );
                        held.deadlines.borrow_mut().push((slot, reservation));
                        Ok(())
                    },
                    |id, serial| {
                        assert!(held.set().is_err());
                        held.tickets.borrow_mut().push_back((id, serial));
                    },
                )
                .unwrap();
            assert_eq!(held.tickets.borrow().len(), 1);
            if close {
                assert!(matches!(
                    poll(&mut request),
                    Poll::Ready(Err(NativeServiceError::Closed))
                ));
                assert!(matches!(held.set(), Err(NativeServiceError::Closed)));
            } else {
                assert!(matches!(
                    poll(&mut request),
                    Poll::Ready(Err(NativeServiceError::Cancelled))
                ));
                assert!(matches!(held.set(), Err(NativeServiceError::ThemeBusy)));
            }
            held.drain();
            assert_eq!(held.applied.get(), 0);
            if close {
                assert!(matches!(held.set(), Err(NativeServiceError::Closed)));
            } else {
                let mut retry = held.set().unwrap();
                held.drain();
                assert!(matches!(poll(&mut retry), Poll::Ready(Ok(_))));
            }
        }
    }

    #[test]
    fn setup_failure_releases_claim_even_after_navigation_removed_pending() {
        for cancel in [false, true] {
            let (_owner, held) = HeldDispatch::new();
            let result = held.controller.set_with(
                ThemeMode::Dark,
                |_slot, _reservation| {
                    if cancel {
                        held.controller.cancel(NativeServiceError::Cancelled, false);
                    }
                    assert!(matches!(held.set(), Err(NativeServiceError::ThemeBusy)));
                    Err(NativeServiceError::Unavailable)
                },
                |_, _| panic!("failed setup must not post a native ticket"),
            );
            assert!(matches!(result, Err(NativeServiceError::Unavailable)));
            assert_eq!(held.controller.active_deadlines.load(Ordering::Acquire), 0);
            assert!(held.controller.state.lock().unwrap().pending.is_none());
            let mut retry = held.set().unwrap();
            held.drain();
            assert!(matches!(poll(&mut retry), Poll::Ready(Ok(_))));
        }
    }

    #[test]
    fn deadline_capacity_failure_releases_unposted_claim_without_running_setup() {
        let (_owner, held) = HeldDispatch::new();
        held.controller
            .active_deadlines
            .store(MAX_DEADLINE_THREADS, Ordering::Release);
        assert!(matches!(
            held.controller.set_with(
                ThemeMode::Dark,
                |_, _| panic!("deadline capacity must precede thread startup"),
                |_, _| panic!("overload must not post a native ticket"),
            ),
            Err(NativeServiceError::Overloaded)
        ));
        assert!(!held.controller.state.lock().unwrap().busy);
        assert!(held.tickets.borrow().is_empty());
        held.controller.active_deadlines.store(0, Ordering::Release);
        let mut retry = held.set().unwrap();
        held.drain();
        assert!(matches!(poll(&mut retry), Poll::Ready(Ok(_))));
    }

    #[test]
    fn completed_future_and_stale_ticket_cannot_release_newer_admission() {
        let (_owner, held) = HeldDispatch::new();
        let mut previous = held.set().unwrap();
        let (_, serial) = *held.tickets.borrow().front().unwrap();
        held.drain();
        assert!(matches!(poll(&mut previous), Poll::Ready(Ok(_))));
        let mut current = held.set().unwrap();
        drop(previous);
        held.controller
            .apply_with(serial, |_| panic!("stale native ticket applied"));
        held.controller
            .cancel_claim(serial, NativeServiceError::Unavailable);
        assert!(matches!(held.set(), Err(NativeServiceError::ThemeBusy)));
        assert_eq!(held.tickets.borrow().len(), 1);
        held.drain();
        assert_eq!(held.applied.get(), 2);
        assert!(matches!(poll(&mut current), Poll::Ready(Ok(_))));
    }

    #[test]
    fn native_failure_and_in_progress_cancellation_release_only_after_native_return() {
        for cancel in [false, true] {
            let (_owner, held) = HeldDispatch::new();
            let mut request = held.set().unwrap();
            held.drain_with(|_| {
                assert!(matches!(held.set(), Err(NativeServiceError::ThemeBusy)));
                if cancel {
                    held.controller.cancel(NativeServiceError::Cancelled, false);
                }
                assert!(matches!(held.set(), Err(NativeServiceError::ThemeBusy)));
                Err(if cancel {
                    NativeServiceError::Cancelled
                } else {
                    NativeServiceError::ThemeUnavailable
                })
            });
            if cancel {
                assert!(matches!(
                    poll(&mut request),
                    Poll::Ready(Err(NativeServiceError::Cancelled))
                ));
            } else {
                assert!(matches!(
                    poll(&mut request),
                    Poll::Ready(Err(NativeServiceError::ThemeUnavailable))
                ));
            }
            let mut retry = held.set().unwrap();
            held.drain();
            assert!(matches!(poll(&mut retry), Poll::Ready(Ok(_))));
        }
    }

    #[test]
    fn revoked_host_cannot_apply_held_work_or_admit_retry() {
        let (owner, held) = HeldDispatch::new();
        let mut request = held.set().unwrap();
        owner.revoke().unwrap();
        held.drain();
        assert_eq!(held.applied.get(), 0);
        assert!(matches!(
            poll(&mut request),
            Poll::Ready(Err(NativeServiceError::Closed))
        ));
        assert!(matches!(held.set(), Err(NativeServiceError::Closed)));
        assert_eq!(held.controller.active_deadlines.load(Ordering::Acquire), 0);
    }

    #[test]
    fn close_cancels_request_and_missing_native_target_discards_ticket() {
        let (_owner, held) = HeldDispatch::new();
        let mut request = held.set().unwrap();
        let ticket = held.tickets.borrow_mut().pop_front().unwrap();
        let registration = Registration {
            id: ticket.0,
            controller: Arc::clone(&held.controller),
            _ui_only: std::marker::PhantomData,
        };
        registration.close();
        held.retire_deadlines();
        // SAFETY: This transfers the production callback's exact ticket type.
        // No native target was registered, so drain cannot access AppKit.
        unsafe { drain(Box::into_raw(Box::new(ticket)).cast::<c_void>()) };
        assert_eq!(held.applied.get(), 0);
        assert!(matches!(
            poll(&mut request),
            Poll::Ready(Err(NativeServiceError::Closed))
        ));
        assert!(matches!(held.set(), Err(NativeServiceError::Closed)));
        assert_eq!(held.controller.active_deadlines.load(Ordering::Acquire), 0);
    }

    #[test]
    fn unattached_window_rejects_without_deadline_or_ticket() {
        let (_owner, held) = HeldDispatch::new();
        held.controller.state.lock().unwrap().target = None;
        assert!(matches!(
            held.set(),
            Err(NativeServiceError::ThemeUnavailable)
        ));
        assert!(held.tickets.borrow().is_empty());
        assert_eq!(held.controller.active_deadlines.load(Ordering::Acquire), 0);
    }

    #[test]
    fn retired_host_rejects_snapshot_admission_and_unpolled_completion() {
        let (owner, lifetime) = crate::HostLifetime::new();
        let controller = Controller::new(crate::EventRegistry::default(), lifetime);
        {
            let mut state = controller.state.lock().unwrap();
            state.target = Some(1);
            state.snapshot = Some(ThemeState {
                mode: ThemeMode::Light,
                dark: false,
            });
        }
        let slot = Arc::new(Slot::new());
        slot.complete(Ok(ThemeState {
            mode: ThemeMode::Light,
            dark: false,
        }));
        let mut request = Request {
            controller: Arc::clone(&controller),
            slot,
        };
        owner.revoke().unwrap();
        assert!(matches!(
            controller.snapshot(),
            Err(NativeServiceError::Closed)
        ));
        assert!(matches!(
            controller.set(ThemeMode::Dark),
            Err(NativeServiceError::Closed)
        ));
        assert!(!controller.observe(true));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(matches!(
            Pin::new(&mut request).poll(&mut cx),
            Poll::Ready(Err(NativeServiceError::Closed))
        ));
    }
}
