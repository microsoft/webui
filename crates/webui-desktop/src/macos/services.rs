// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::path::Path;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll, Waker};

#[cfg(feature = "native-dialogs")]
use block2::RcBlock;

#[cfg(feature = "native-dialogs")]
use objc2::rc::Retained;
use objc2::rc::Weak as ObjcWeak;
#[cfg(feature = "native-dialogs")]
use objc2::MainThreadMarker;
use objc2_app_kit::NSWindow;
#[cfg(feature = "native-dialogs")]
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertSecondButtonReturn, NSAlertStyle, NSModalResponse,
    NSModalResponseCancel,
};
#[cfg(feature = "native-dialogs")]
use objc2_foundation::NSString;
use objc2_web_kit::WKWebView;

use super::{ContentGeometry, GeometryRequest, Inner, NativeServiceError, ScreenRectPoints};

const MAX_GEOMETRY_READS: usize = 16;
static NEXT_TARGET: AtomicU64 = AtomicU64::new(1);

thread_local! {
    // Only this UI-local registry sees Objective-C objects. Cross-thread
    // callers and GCD messages carry opaque IDs and Send + Sync state.
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
    dispatch: Arc<Dispatch>,
    window: ObjcWeak<NSWindow>,
    view: ObjcWeak<WKWebView>,
    owner: Weak<Inner>,
    #[cfg(feature = "native-dialogs")]
    active_dialog: RefCell<Option<ActiveDialog>>,
}

pub(crate) struct Dispatch {
    id: u64,
    closed: AtomicBool,
    queued: AtomicBool,
    pending: Mutex<Vec<Pending>>,
    #[cfg(feature = "native-dialogs")]
    dialog: Mutex<Option<DialogSubmission>>,
    #[cfg(feature = "native-dialogs")]
    cancel_dialog_id: AtomicU64,
}

#[cfg(feature = "native-dialogs")]
struct DialogSubmission {
    owner: Arc<crate::native_dialogs::DialogState>,
    id: u64,
    epoch: u64,
    copy: crate::native_dialogs::DialogCopy,
    signal: Arc<crate::native_dialogs::Signal>,
}

#[cfg(feature = "native-dialogs")]
struct ActiveDialog {
    id: u64,
    alert: Retained<NSAlert>,
    owner: Arc<crate::native_dialogs::DialogState>,
}

struct Pending {
    slot: Weak<GeometrySlot>,
    epoch: u64,
}

struct SlotState {
    result: Option<Result<ContentGeometry, NativeServiceError>>,
    waker: Option<Waker>,
}

pub(super) struct GeometrySlot(Mutex<SlotState>);

impl GeometrySlot {
    #[cfg(test)]
    pub(super) fn test_slot() -> Self {
        Self(Mutex::new(SlotState {
            result: None,
            waker: None,
        }))
    }

    pub(super) fn complete(&self, result: Result<ContentGeometry, NativeServiceError>) {
        let waker = if let Ok(mut state) = self.0.lock() {
            state.result = Some(result);
            state.waker.take()
        } else {
            None
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    pub(super) fn poll(
        &self,
        cx: &mut Context<'_>,
        owner: &Inner,
    ) -> Poll<Result<ContentGeometry, NativeServiceError>> {
        let Ok(mut state) = self.0.lock() else {
            return Poll::Ready(Err(NativeServiceError::Unavailable));
        };
        if let Some(result) = state.result.take() {
            return Poll::Ready(match result {
                Ok(snapshot)
                    if snapshot.revision != owner.geometry_revision.load(Ordering::Acquire) =>
                {
                    Err(NativeServiceError::Stale)
                }
                other => other,
            });
        }
        state.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

/// UI-thread registration for one exact window generation. Does not retain the
/// native window/view; dropping it retires outstanding reads without waiting.
pub(crate) struct Registration {
    dispatch: Arc<Dispatch>,
    _ui_only: std::marker::PhantomData<Rc<()>>,
}

impl Registration {
    pub(crate) fn close(&self) {
        if !self.dispatch.closed.swap(true, Ordering::AcqRel) {
            self.dispatch.cancel_all(NativeServiceError::Closed);
            #[cfg(feature = "native-dialogs")]
            self.dispatch.cancel_queued_dialog();
        }
        let target = TARGETS.with(|targets| targets.borrow_mut().remove(&self.dispatch.id));
        #[cfg(feature = "native-dialogs")]
        if let Some(target) = &target {
            if let Some(owner) = target.owner.upgrade() {
                owner.dialogs.notify_closed();
            }
            let active = target
                .active_dialog
                .borrow()
                .as_ref()
                .map(|active| active.alert.clone());
            if let (Some(alert), Some(window)) = (active, target.window.load()) {
                window.endSheet_returnCode(&alert.window(), NSModalResponseCancel);
            }
        }
        drop(target);
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.close();
    }
}

pub(super) fn install(owner: &Arc<Inner>, window: &NSWindow, view: &WKWebView) -> Registration {
    let dispatch = Arc::new(Dispatch {
        id: NEXT_TARGET.fetch_add(1, Ordering::Relaxed),
        closed: AtomicBool::new(false),
        queued: AtomicBool::new(false),
        pending: Mutex::new(Vec::new()),
        #[cfg(feature = "native-dialogs")]
        dialog: Mutex::new(None),
        #[cfg(feature = "native-dialogs")]
        cancel_dialog_id: AtomicU64::new(0),
    });
    let target = Rc::new(Target {
        dispatch: Arc::clone(&dispatch),
        window: ObjcWeak::new(window),
        view: ObjcWeak::new(view),
        owner: Arc::downgrade(owner),
        #[cfg(feature = "native-dialogs")]
        active_dialog: RefCell::new(None),
    });
    TARGETS.with(|targets| {
        targets.borrow_mut().insert(dispatch.id, target);
    });
    if let Ok(mut active) = owner.geometry_dispatch.lock() {
        *active = Some(Arc::clone(&dispatch));
    }
    #[cfg(feature = "native-dialogs")]
    owner.dialogs.attach(Arc::clone(&dispatch));
    Registration {
        dispatch,
        _ui_only: std::marker::PhantomData,
    }
}

pub(super) fn request(owner: &Arc<Inner>) -> Result<GeometryRequest, NativeServiceError> {
    if owner.closed.load(Ordering::Acquire) || !owner.lifetime.is_active() {
        return Err(NativeServiceError::Closed);
    }
    let epoch = owner.document_epoch.load(Ordering::Acquire);
    if epoch == 0 || owner.committed_epoch.load(Ordering::Acquire) != epoch {
        return Err(NativeServiceError::GeometryUnavailable);
    }
    let dispatch = owner
        .geometry_dispatch
        .lock()
        .map_err(|_| NativeServiceError::Unavailable)?
        .as_ref()
        .cloned()
        .ok_or(NativeServiceError::GeometryUnavailable)?;
    let slot = Arc::new(GeometrySlot(Mutex::new(SlotState {
        result: None,
        waker: None,
    })));
    dispatch.enqueue(Pending {
        slot: Arc::downgrade(&slot),
        epoch,
    })?;
    Ok(GeometryRequest {
        inner: Arc::clone(owner),
        epoch,
        slot,
    })
}

pub(super) fn cancel_pending(owner: &Inner, close: bool) {
    let dispatch = owner
        .geometry_dispatch
        .lock()
        .ok()
        .and_then(|active| active.as_ref().cloned());
    if let Some(dispatch) = dispatch {
        if close {
            dispatch.closed.store(true, Ordering::Release);
            dispatch.cancel_all(NativeServiceError::Closed);
        } else {
            dispatch.cancel_all(NativeServiceError::Stale);
        }
    }
}

impl Dispatch {
    #[cfg(feature = "native-dialogs")]
    pub(crate) fn enqueue_dialog(
        &self,
        owner: Arc<crate::native_dialogs::DialogState>,
        (id, epoch): (u64, u64),
        copy: crate::native_dialogs::DialogCopy,
        signal: Arc<crate::native_dialogs::Signal>,
    ) -> Result<(), crate::DialogError> {
        let mut pending = self
            .dialog
            .lock()
            .map_err(|_| crate::DialogError::Unavailable)?;
        if self.closed.load(Ordering::Acquire) {
            return Err(crate::DialogError::Closed);
        }
        if pending.is_some() {
            return Err(crate::DialogError::Busy);
        }
        *pending = Some(DialogSubmission {
            owner,
            id,
            epoch,
            copy,
            signal,
        });
        drop(pending);
        self.wake();
        Ok(())
    }

    #[cfg(feature = "native-dialogs")]
    pub(crate) fn cancel_dialog(&self, id: u64) {
        self.cancel_dialog_id.store(id, Ordering::Release);
        self.wake();
    }

    #[cfg(feature = "native-dialogs")]
    fn cancel_queued_dialog(&self) {
        let pending = self.dialog.lock().ok().and_then(|mut slot| slot.take());
        if let Some(pending) = pending {
            pending
                .owner
                .complete(pending.id, Err(crate::DialogError::Closed));
        }
    }

    fn wake(&self) {
        if !self.queued.swap(true, Ordering::AcqRel) {
            let context = Box::into_raw(Box::new(self.id)).cast::<c_void>();
            // SAFETY: GCD delivers the opaque ID once on the main queue. Only
            // the registered UI-local target may access its AppKit objects.
            unsafe {
                dispatch_async_f(
                    std::ptr::addr_of!(_dispatch_main_q).cast_mut(),
                    context,
                    drain,
                );
            }
        }
    }

    fn enqueue(&self, pending: Pending) -> Result<(), NativeServiceError> {
        let mut queue = self
            .pending
            .lock()
            .map_err(|_| NativeServiceError::Unavailable)?;
        if self.closed.load(Ordering::Acquire) {
            return Err(NativeServiceError::Closed);
        }
        if queue.len() >= MAX_GEOMETRY_READS {
            return Err(NativeServiceError::Overloaded);
        }
        queue.push(pending);
        drop(queue);
        self.wake();
        Ok(())
    }

    fn cancel_all(&self, error: NativeServiceError) {
        let pending = self
            .pending
            .lock()
            .map(|mut queue| std::mem::take(&mut *queue));
        if let Ok(pending) = pending {
            for item in pending {
                if let Some(slot) = item.slot.upgrade() {
                    slot.complete(Err(match error {
                        NativeServiceError::Closed => NativeServiceError::Closed,
                        _ => NativeServiceError::Stale,
                    }));
                }
            }
        }
    }
}

impl Target {
    #[cfg(feature = "native-dialogs")]
    fn drain_dialog(this: &Rc<Self>) {
        let cancel_id = this.dispatch.cancel_dialog_id.swap(0, Ordering::AcqRel);
        let active = this
            .active_dialog
            .borrow()
            .as_ref()
            .filter(|active| active.id == cancel_id)
            .map(|active| active.alert.clone());
        if let (Some(alert), Some(window)) = (active, this.window.load()) {
            window.endSheet_returnCode(&alert.window(), NSModalResponseCancel);
        }
        let pending = this
            .dispatch
            .dialog
            .lock()
            .ok()
            .and_then(|mut slot| slot.take());
        if let Some(pending) = pending {
            Self::start_dialog(this, pending);
        }
    }

    #[cfg(feature = "native-dialogs")]
    fn start_dialog(this: &Rc<Self>, pending: DialogSubmission) {
        let fail = |error| pending.owner.complete(pending.id, Err(error));
        if this.dispatch.closed.load(Ordering::Acquire)
            || !pending.owner.current(pending.id, pending.epoch)
        {
            fail(crate::DialogError::Navigated);
            return;
        }
        let (Some(window), Some(view), Some(mtm)) = (
            this.window.load(),
            this.view.load(),
            MainThreadMarker::new(),
        ) else {
            fail(crate::DialogError::Closed);
            return;
        };
        if !view
            .window()
            .is_some_and(|attached| std::ptr::eq(&*attached, &*window))
        {
            fail(crate::DialogError::Closed);
            return;
        }
        if window.attachedSheet().is_some() || this.active_dialog.borrow().is_some() {
            fail(crate::DialogError::Busy);
            return;
        }
        let alert = NSAlert::new(mtm);
        alert.setAlertStyle(NSAlertStyle::Warning);
        alert.setMessageText(&NSString::from_str(pending.copy.title()));
        alert.setInformativeText(&NSString::from_str(pending.copy.message()));
        let order = pending.copy.mac_buttons();
        for button in order {
            let Some(label) = pending.copy.label(*button) else {
                fail(crate::DialogError::Unavailable);
                return;
            };
            alert.addButtonWithTitle(&NSString::from_str(label));
        }
        let id = pending.id;
        let copy = pending.copy;
        let weak = Rc::downgrade(this);
        let callback = RcBlock::new(move |response: NSModalResponse| {
            let Some(target) = weak.upgrade() else { return };
            let active = {
                let mut slot = target.active_dialog.borrow_mut();
                if slot.as_ref().is_some_and(|active| active.id == id) {
                    slot.take()
                } else {
                    None
                }
            };
            let Some(active) = active else { return };
            let outcome = if response == NSAlertFirstButtonReturn {
                Ok(copy.outcome(order[0]))
            } else if response == NSAlertSecondButtonReturn {
                Ok(order
                    .get(1)
                    .copied()
                    .map_or(crate::DialogOutcome::Cancelled, |button| {
                        copy.outcome(button)
                    }))
            } else if response == NSModalResponseCancel {
                Ok(crate::DialogOutcome::Cancelled)
            } else {
                Err(crate::DialogError::Os {
                    operation: "NSAlert(response)",
                    code: i32::try_from(response).unwrap_or(i32::MAX),
                })
            };
            active.owner.complete(id, outcome);
        });
        *this.active_dialog.borrow_mut() = Some(ActiveDialog {
            id,
            alert: alert.clone(),
            owner: Arc::clone(&pending.owner),
        });
        if pending.signal.cancelled.load(Ordering::Acquire)
            || !pending.owner.current(id, pending.epoch)
        {
            this.active_dialog.borrow_mut().take();
            pending
                .owner
                .complete(id, Err(crate::DialogError::Navigated));
            return;
        }
        alert.beginSheetModalForWindow_completionHandler(&window, Some(&callback));
    }
}

unsafe extern "C" fn drain(context: *mut c_void) {
    // SAFETY: enqueue transfers one boxed ID to this GCD callback.
    let id = unsafe { Box::from_raw(context.cast::<u64>()) };
    let target = TARGETS.with(|targets| targets.borrow().get(&id).cloned());
    if let Some(target) = target {
        target.dispatch.queued.store(false, Ordering::Release);
        let pending = target
            .dispatch
            .pending
            .lock()
            .map(|mut queue| std::mem::take(&mut *queue));
        if let Ok(pending) = pending {
            for item in pending {
                if let Some(slot) = item.slot.upgrade() {
                    slot.complete(snapshot(&target, item.epoch));
                }
            }
        }
        #[cfg(feature = "native-dialogs")]
        Target::drain_dialog(&target);
    }
}

fn snapshot(target: &Target, epoch: u64) -> Result<ContentGeometry, NativeServiceError> {
    let owner = target.owner.upgrade().ok_or(NativeServiceError::Closed)?;
    if owner.closed.load(Ordering::Acquire) || !owner.lifetime.is_active() {
        return Err(NativeServiceError::Closed);
    }
    if epoch != owner.document_epoch.load(Ordering::Acquire)
        || epoch != owner.committed_epoch.load(Ordering::Acquire)
    {
        return Err(NativeServiceError::Stale);
    }
    let view = target
        .view
        .load()
        .ok_or(NativeServiceError::GeometryUnavailable)?;
    let window = target
        .window
        .load()
        .ok_or(NativeServiceError::GeometryUnavailable)?;
    // A finished main document, not merely a provisional navigation or an
    // old WebKit completion, is required for an authoritative native snapshot.
    // SAFETY: GCD drains on the main queue; WebKit getters require that thread.
    if unsafe { view.isLoading() } {
        return Err(NativeServiceError::GeometryUnavailable);
    }
    let attached = view
        .window()
        .is_some_and(|attached| std::ptr::eq(&*attached, &*window));
    if !attached {
        return Err(NativeServiceError::GeometryUnavailable);
    }
    let rect = window.convertRectToScreen(view.convertRect_toView(view.bounds(), None));
    let backing_scale = window.backingScaleFactor();
    // SAFETY: WKWebView getters are read on its native main thread.
    let (page_zoom, magnification) = unsafe { (view.pageZoom(), view.magnification()) };
    let components = [
        rect.origin.x,
        rect.origin.y,
        rect.size.width,
        rect.size.height,
        backing_scale,
        page_zoom,
        magnification,
    ];
    if components.iter().any(|value| !value.is_finite())
        || rect.size.width <= 0.0
        || rect.size.height <= 0.0
        || backing_scale <= 0.0
        || page_zoom <= 0.0
        || magnification <= 0.0
    {
        return Err(NativeServiceError::GeometryUnavailable);
    }
    let page_changed = owner
        .last_page_zoom
        .swap(page_zoom.to_bits(), Ordering::AcqRel)
        != page_zoom.to_bits();
    let magnification_changed = owner
        .last_magnification
        .swap(magnification.to_bits(), Ordering::AcqRel)
        != magnification.to_bits();
    if page_changed || magnification_changed {
        owner.geometry_revision.fetch_add(1, Ordering::AcqRel);
    }
    if owner.closed.load(Ordering::Acquire)
        || !owner.lifetime.is_active()
        || epoch != owner.document_epoch.load(Ordering::Acquire)
    {
        return Err(NativeServiceError::Stale);
    }
    Ok(ContentGeometry {
        window_generation: owner.window_generation,
        document_epoch: epoch,
        revision: owner.geometry_revision.load(Ordering::Acquire),
        screen_rect: ScreenRectPoints {
            x: rect.origin.x,
            y: rect.origin.y,
            width: rect.size.width,
            height: rect.size.height,
        },
        backing_scale,
        page_zoom,
        magnification,
    })
}

pub(super) fn open_url(url: &str) -> Result<(), NativeServiceError> {
    // Fixed OS binary and argv: no shell, interpolation, or renderer path.
    open(url)
}

pub(super) fn open_document(path: &Path) -> Result<(), NativeServiceError> {
    open(path.as_os_str())
}

fn open(target: impl AsRef<std::ffi::OsStr>) -> Result<(), NativeServiceError> {
    let status = Command::new("/usr/bin/open")
        .arg("--")
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| NativeServiceError::Os(error.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(NativeServiceError::Os(format!(
            "system opener exited with {status}"
        )))
    }
}
