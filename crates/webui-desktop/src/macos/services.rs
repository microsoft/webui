// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
#[cfg(feature = "native-picker")]
use std::ffi::{CString, OsStr};
#[cfg(feature = "native-picker")]
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll, Waker};

#[cfg(any(feature = "native-dialogs", feature = "native-picker"))]
use block2::RcBlock;
#[cfg(any(feature = "native-dialogs", feature = "native-picker"))]
use objc2::rc::Retained;
use objc2::rc::Weak as ObjcWeak;
#[cfg(any(feature = "native-dialogs", feature = "native-picker"))]
use objc2::MainThreadMarker;
use objc2_app_kit::NSWindow;
#[cfg(feature = "native-dialogs")]
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSAlertSecondButtonReturn, NSAlertStyle};
#[cfg(any(feature = "native-dialogs", feature = "native-picker"))]
use objc2_app_kit::{NSModalResponse, NSModalResponseCancel};
#[cfg(feature = "native-picker")]
use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
#[cfg(any(feature = "native-dialogs", feature = "native-picker"))]
use objc2_foundation::NSString;
#[cfg(feature = "native-picker")]
use objc2_foundation::NSURL;
use objc2_web_kit::WKWebView;

#[cfg(feature = "native-picker")]
use super::PickerPermit;
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
    #[cfg(feature = "native-picker")]
    active_picker: RefCell<Option<ActivePicker>>,
    #[cfg(feature = "native-dialogs")]
    active_dialog: RefCell<Option<ActiveDialog>>,
}

pub(crate) struct Dispatch {
    id: u64,
    closed: AtomicBool,
    queued: AtomicBool,
    pending: Mutex<Vec<Pending>>,
    #[cfg(feature = "native-picker")]
    picker: Mutex<Option<PickerSubmission>>,
    #[cfg(feature = "native-picker")]
    cancel_picker_id: AtomicU64,
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

#[cfg(feature = "native-picker")]
pub(super) struct PickerSubmission {
    pub(super) id: u64,
    pub(super) generation: u64,
    pub(super) title: Option<String>,
    pub(super) initial: Option<std::path::PathBuf>,
    pub(super) slot: Weak<PickerSlot>,
    pub(super) permit: Arc<PickerPermit>,
}

#[cfg(feature = "native-picker")]
struct ActivePicker {
    id: u64,
    generation: u64,
    panel: Retained<NSOpenPanel>,
    slot: Weak<PickerSlot>,
    _permit: Arc<PickerPermit>,
}

#[cfg(feature = "native-picker")]
struct PickerState {
    result: Option<Result<Option<std::path::PathBuf>, NativeServiceError>>,
    waker: Option<Waker>,
}

#[cfg(feature = "native-picker")]
pub(super) struct PickerSlot(Mutex<PickerState>);

#[cfg(feature = "native-picker")]
impl PickerSlot {
    pub(super) fn new() -> Self {
        Self(Mutex::new(PickerState {
            result: None,
            waker: None,
        }))
    }

    fn complete(&self, result: Result<Option<std::path::PathBuf>, NativeServiceError>) {
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

    #[cfg(test)]
    pub(super) fn test_complete(
        &self,
        result: Result<Option<std::path::PathBuf>, NativeServiceError>,
    ) {
        self.complete(result);
    }

    pub(super) fn poll(
        &self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Option<std::path::PathBuf>, NativeServiceError>> {
        let Ok(mut state) = self.0.lock() else {
            return Poll::Ready(Err(NativeServiceError::Unavailable));
        };
        if let Some(result) = state.result.take() {
            return Poll::Ready(result);
        }
        state.waker = Some(cx.waker().clone());
        Poll::Pending
    }
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
            #[cfg(feature = "native-picker")]
            self.dispatch.cancel_queued_picker(None, true);
            #[cfg(feature = "native-dialogs")]
            self.dispatch.cancel_queued_dialog();
        }
        let target = TARGETS.with(|targets| targets.borrow_mut().remove(&self.dispatch.id));
        #[cfg(any(feature = "native-dialogs", feature = "native-picker"))]
        if let Some(target) = &target {
            #[cfg(feature = "native-dialogs")]
            {
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
            #[cfg(feature = "native-picker")]
            let active = target.active_picker.borrow_mut().take();
            #[cfg(feature = "native-picker")]
            if let Some(active) = active {
                if let Some(slot) = active.slot.upgrade() {
                    slot.complete(Err(NativeServiceError::Closed));
                }
                // SAFETY: Registration is UI-local; AppKit owns the open sheet.
                unsafe { active.panel.cancel(None) };
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
        #[cfg(feature = "native-picker")]
        picker: Mutex::new(None),
        #[cfg(feature = "native-picker")]
        cancel_picker_id: AtomicU64::new(0),
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
        #[cfg(feature = "native-picker")]
        active_picker: RefCell::new(None),
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

#[cfg(feature = "native-picker")]
pub(super) fn require_picker_window(owner: &Inner) -> Result<(), NativeServiceError> {
    let active = owner
        .geometry_dispatch
        .lock()
        .map_err(|_| NativeServiceError::Unavailable)?;
    if active
        .as_ref()
        .is_none_or(|dispatch| dispatch.closed.load(Ordering::Acquire))
    {
        return Err(NativeServiceError::Unavailable);
    }
    Ok(())
}

#[cfg(feature = "native-picker")]
pub(super) fn enqueue_picker(
    owner: &Inner,
    submission: PickerSubmission,
) -> Result<(), NativeServiceError> {
    let dispatch = owner
        .geometry_dispatch
        .lock()
        .map_err(|_| NativeServiceError::Unavailable)?
        .as_ref()
        .cloned()
        .ok_or(NativeServiceError::Unavailable)?;
    let mut picker = dispatch
        .picker
        .lock()
        .map_err(|_| NativeServiceError::Unavailable)?;
    if dispatch.closed.load(Ordering::Acquire) || !owner.lifetime.is_active() {
        return Err(NativeServiceError::Closed);
    }
    if picker.is_some() {
        return Err(NativeServiceError::PickerBusy);
    }
    *picker = Some(submission);
    drop(picker);
    dispatch.wake();
    Ok(())
}

/// This is thread-safe: panel cancellation itself is posted to the main
/// queue, never performed on a host worker or while holding an SDK lock.
#[cfg(feature = "native-picker")]
pub(super) fn cancel_picker(owner: &Inner, id: u64) {
    let dispatch = owner
        .geometry_dispatch
        .lock()
        .ok()
        .and_then(|active| active.as_ref().cloned());
    if let Some(dispatch) = dispatch {
        dispatch.cancel_queued_picker(Some(id), false);
        dispatch.cancel_picker_id.store(id, Ordering::Release);
        dispatch.wake();
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

    #[cfg(feature = "native-picker")]
    fn cancel_queued_picker(&self, id: Option<u64>, close: bool) {
        let queued = self.picker.lock().ok().and_then(|mut picker| {
            if picker
                .as_ref()
                .is_some_and(|pending| id.is_none_or(|id| id == pending.id))
            {
                picker.take()
            } else {
                None
            }
        });
        if let Some(queued) = queued {
            if let Some(slot) = queued.slot.upgrade() {
                slot.complete(Err(if close {
                    NativeServiceError::Closed
                } else {
                    NativeServiceError::Cancelled
                }));
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

    #[cfg(feature = "native-picker")]
    fn drain_picker(this: &Rc<Self>) {
        let canceled = this.dispatch.cancel_picker_id.swap(0, Ordering::AcqRel);
        if let Some(panel) = this
            .active_picker
            .borrow()
            .as_ref()
            .filter(|active| active.id == canceled)
            .map(|active| active.panel.clone())
        {
            // SAFETY: GCD invokes this on AppKit's main queue; a native
            // response (not mere queue admission) completes the picker.
            unsafe { panel.cancel(None) };
        }
        let pending = this
            .dispatch
            .picker
            .lock()
            .ok()
            .and_then(|mut picker| picker.take());
        if let Some(pending) = pending {
            Self::start_picker(this, pending);
        }
    }

    #[cfg(feature = "native-picker")]
    fn start_picker(this: &Rc<Self>, pending: PickerSubmission) {
        let Some(slot) = pending.slot.upgrade() else {
            return;
        };
        let Some(owner) = this.owner.upgrade() else {
            slot.complete(Err(NativeServiceError::Closed));
            return;
        };
        if this.dispatch.closed.load(Ordering::Acquire)
            || owner.closed.load(Ordering::Acquire)
            || !owner.lifetime.is_active()
        {
            slot.complete(Err(NativeServiceError::Closed));
            return;
        }
        if owner.generation.load(Ordering::Acquire) != pending.generation {
            slot.complete(Err(NativeServiceError::Cancelled));
            return;
        }
        if this.active_picker.borrow().is_some() {
            slot.complete(Err(NativeServiceError::PickerBusy));
            return;
        }
        let Some(window) = this.window.load() else {
            slot.complete(Err(NativeServiceError::Closed));
            return;
        };
        let Some(mtm) = MainThreadMarker::new() else {
            slot.complete(Err(NativeServiceError::Unavailable));
            return;
        };
        let panel = NSOpenPanel::openPanel(mtm);
        panel.setCanChooseDirectories(true);
        panel.setCanChooseFiles(false);
        panel.setAllowsMultipleSelection(false);
        if let Some(title) = pending.title.as_deref() {
            panel.setTitle(Some(&NSString::from_str(title)));
        }
        if let Some(path) = pending.initial.as_ref() {
            let Ok(bytes) = CString::new(path.as_os_str().as_bytes()) else {
                slot.complete(Err(NativeServiceError::InvalidInitialDirectory));
                return;
            };
            let Some(pointer) = std::ptr::NonNull::new(bytes.as_ptr().cast_mut()) else {
                slot.complete(Err(NativeServiceError::InvalidInitialDirectory));
                return;
            };
            // SAFETY: CString lives through the NSURL call; it preserves
            // non-UTF-8 filesystem bytes without a lossy NSString conversion.
            let url = unsafe {
                NSURL::fileURLWithFileSystemRepresentation_isDirectory_relativeToURL(
                    pointer, true, None,
                )
            };
            panel.setDirectoryURL(Some(&url));
        }
        let id = pending.id;
        let weak_target = Rc::downgrade(this);
        let callback = RcBlock::new(move |response: NSModalResponse| {
            let Some(target) = weak_target.upgrade() else {
                return;
            };
            let active = {
                let mut current = target.active_picker.borrow_mut();
                if current.as_ref().is_some_and(|active| active.id == id) {
                    current.take()
                } else {
                    None
                }
            };
            let Some(active) = active else { return };
            active.panel.orderOut(None);
            let result = target.picker_response(&active, response);
            let slot = active.slot.upgrade();
            let permit = Arc::clone(&active._permit);
            complete_picker_after_release(&permit, active, slot, result);
        });
        *this.active_picker.borrow_mut() = Some(ActivePicker {
            id,
            generation: pending.generation,
            panel: panel.clone(),
            slot: pending.slot,
            _permit: pending.permit,
        });
        panel.beginSheetModalForWindow_completionHandler(&window, &callback);
    }

    #[cfg(feature = "native-picker")]
    fn picker_response(
        &self,
        active: &ActivePicker,
        response: NSModalResponse,
    ) -> Result<Option<std::path::PathBuf>, NativeServiceError> {
        let owner = self.owner.upgrade().ok_or(NativeServiceError::Closed)?;
        if self.dispatch.closed.load(Ordering::Acquire)
            || owner.closed.load(Ordering::Acquire)
            || !owner.lifetime.is_active()
        {
            return Err(NativeServiceError::Closed);
        }
        if owner.generation.load(Ordering::Acquire) != active.generation {
            return Err(NativeServiceError::Cancelled);
        }
        if response == NSModalResponseCancel {
            return Ok(None);
        }
        if response != NSModalResponseOK {
            return Err(NativeServiceError::PickerFailure(format!(
                "OS returned modal response {response:?}"
            )));
        }
        let urls = active.panel.URLs();
        if urls.count() != 1 {
            return Err(NativeServiceError::InvalidSelection);
        }
        let url = urls
            .firstObject()
            .ok_or(NativeServiceError::InvalidSelection)?;
        selected_directory(&url).map(Some)
    }
}

#[cfg(feature = "native-picker")]
fn selected_directory(url: &NSURL) -> Result<std::path::PathBuf, NativeServiceError> {
    if !url.isFileURL() {
        return Err(NativeServiceError::InvalidSelection);
    }
    let pointer = url.fileSystemRepresentation();
    let limit = super::MAX_NATIVE_DOCUMENT_PATH_BYTES;
    // SAFETY: NSURL retains a NUL-terminated filesystem representation while
    // `url` lives. Inspect at most the permitted bytes plus one terminator
    // before constructing a Rust path; never scan or copy an unbounded URL.
    let length = unsafe { libc::strnlen(pointer.as_ptr(), limit + 1) };
    if length == 0 || length > limit {
        return Err(NativeServiceError::InvalidSelection);
    }
    // SAFETY: strnlen found a terminator within the checked NSURL buffer;
    // these preceding bytes remain valid for this synchronous copy.
    let bytes = unsafe { std::slice::from_raw_parts(pointer.as_ptr().cast::<u8>(), length) };
    let path = std::path::PathBuf::from(OsStr::from_bytes(bytes));
    if !path.is_absolute() {
        return Err(NativeServiceError::InvalidSelection);
    }
    Ok(path)
}

#[cfg(feature = "native-picker")]
fn complete_picker_after_release<T>(
    permit: &Arc<PickerPermit>,
    native_reservation: T,
    slot: Option<Arc<PickerSlot>>,
    result: Result<Option<std::path::PathBuf>, NativeServiceError>,
) {
    // The native panel and its in-flight permit must leave scope before
    // waking the Rust host. A resumed handler may immediately ask for the
    // next picker without observing a spurious PickerBusy.
    permit.release();
    drop(native_reservation);
    if let Some(slot) = slot {
        slot.complete(result);
    }
}

#[cfg(all(test, feature = "native-picker"))]
mod picker_completion_tests {
    use super::*;
    use std::task::{Wake, Waker};

    #[test]
    fn selected_file_url_is_bounded_before_path_copy() -> Result<(), Box<dyn std::error::Error>> {
        let folder = tempfile::tempdir()?;
        let path = CString::new(folder.path().as_os_str().as_bytes())?;
        let pointer = std::ptr::NonNull::new(path.as_ptr().cast_mut()).ok_or("null path")?;
        // SAFETY: The CString remains live for this Foundation constructor.
        let url = unsafe {
            NSURL::fileURLWithFileSystemRepresentation_isDirectory_relativeToURL(
                pointer, true, None,
            )
        };
        assert_eq!(selected_directory(&url)?, folder.path());

        let remote = NSURL::URLWithString(&NSString::from_str("https://example.invalid/"))
            .ok_or("invalid remote fixture URL")?;
        assert!(matches!(
            selected_directory(&remote),
            Err(NativeServiceError::InvalidSelection)
        ));

        let overlong = CString::new(format!(
            "/{}",
            "x".repeat(super::super::MAX_NATIVE_DOCUMENT_PATH_BYTES + 1)
        ))?;
        let pointer =
            std::ptr::NonNull::new(overlong.as_ptr().cast_mut()).ok_or("null long path")?;
        // SAFETY: The CString remains live and contains the entire oversized
        // representation through this Foundation constructor.
        let url = unsafe {
            NSURL::fileURLWithFileSystemRepresentation_isDirectory_relativeToURL(
                pointer, true, None,
            )
        };
        assert!(matches!(
            selected_directory(&url),
            Err(NativeServiceError::InvalidSelection)
        ));
        Ok(())
    }

    struct ReleaseFlag(Arc<AtomicBool>);
    impl Drop for ReleaseFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    struct CheckWake(Arc<AtomicBool>, Arc<Inner>);
    impl Wake for CheckWake {
        fn wake(self: Arc<Self>) {
            assert!(self.0.load(Ordering::Acquire));
            assert!(!self.1.picker_busy.load(Ordering::Acquire));
        }

        fn wake_by_ref(self: &Arc<Self>) {
            assert!(self.0.load(Ordering::Acquire));
            assert!(!self.1.picker_busy.load(Ordering::Acquire));
        }
    }

    #[test]
    fn panel_permit_is_released_before_completion_wakes_next_request() {
        let released = Arc::new(AtomicBool::new(false));
        let (_host, lifetime) = crate::HostLifetime::new();
        let events = crate::EventRegistry::default();
        let service =
            match crate::native_services::NativeServices::new(&events, Arc::default(), lifetime) {
                Ok(service) => service,
                Err(error) => panic!("fixture setup: {error}"),
            };
        let owner = Arc::clone(&service.0);
        owner.picker_busy.store(true, Ordering::Release);
        let permit = Arc::new(PickerPermit::new(Arc::clone(&owner)));
        let future_permit = Arc::clone(&permit);
        let slot = Arc::new(PickerSlot::new());
        let waker = Waker::from(Arc::new(CheckWake(
            Arc::clone(&released),
            Arc::clone(&owner),
        )));
        let mut context = Context::from_waker(&waker);
        assert!(matches!(slot.poll(&mut context), Poll::Pending));
        complete_picker_after_release(
            &permit,
            ReleaseFlag(Arc::clone(&released)),
            Some(Arc::clone(&slot)),
            Ok(None),
        );
        assert!(released.load(Ordering::Acquire));
        assert!(!owner.picker_busy.load(Ordering::Acquire));
        assert!(matches!(slot.poll(&mut context), Poll::Ready(Ok(None))));
        // The awaitable's later Drop must not clear the next admission.
        owner.picker_busy.store(true, Ordering::Release);
        drop(future_permit);
        drop(permit);
        assert!(owner.picker_busy.load(Ordering::Acquire));
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
        #[cfg(feature = "native-picker")]
        Target::drain_picker(&target);
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
