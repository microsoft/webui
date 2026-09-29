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

use objc2::rc::Weak as ObjcWeak;
use objc2_app_kit::NSWindow;
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
}

pub(super) struct Dispatch {
    id: u64,
    closed: AtomicBool,
    queued: AtomicBool,
    pending: Mutex<Vec<Pending>>,
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
        }
        let target = TARGETS.with(|targets| targets.borrow_mut().remove(&self.dispatch.id));
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
    });
    let target = Rc::new(Target {
        dispatch: Arc::clone(&dispatch),
        window: ObjcWeak::new(window),
        view: ObjcWeak::new(view),
        owner: Arc::downgrade(owner),
    });
    TARGETS.with(|targets| {
        targets.borrow_mut().insert(dispatch.id, target);
    });
    if let Ok(mut active) = owner.geometry_dispatch.lock() {
        *active = Some(Arc::clone(&dispatch));
    }
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
        if !self.queued.swap(true, Ordering::AcqRel) {
            let context = Box::into_raw(Box::new(self.id)).cast::<c_void>();
            // SAFETY: GCD delivers the owned ID once on the main queue; the
            // registry looks up only the exact live window generation.
            unsafe {
                dispatch_async_f(
                    std::ptr::addr_of!(_dispatch_main_q).cast_mut(),
                    context,
                    drain,
                );
            }
        }
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
