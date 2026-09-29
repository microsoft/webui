// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! UI-local native clipboard queue. Production and tests call the same
//! pasteboard write/readback helper; tests never touch generalPasteboard.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

#[cfg(test)]
use objc2::rc::Retained;
use objc2_app_kit::{NSPasteboard, NSPasteboardTypePNG};
use objc2_foundation::NSData;

use crate::clipboard::{ClipboardError, ClipboardState};

static NEXT_TARGET: AtomicU64 = AtomicU64::new(1);
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
    owner: Weak<ClipboardState>,
    dispatch: Arc<Dispatch>,
    #[cfg(test)]
    private_board: Option<Retained<NSPasteboard>>,
}

pub(crate) struct Dispatch {
    id: u64,
    closed: AtomicBool,
    queued: AtomicBool,
    pending: Mutex<Option<u64>>,
    #[cfg(test)]
    manual: bool,
}

pub(crate) struct Registration {
    dispatch: Arc<Dispatch>,
    owner: Arc<ClipboardState>,
    _ui_only: std::marker::PhantomData<Rc<()>>,
}

impl Registration {
    pub(crate) fn close(&self) {
        self.dispatch.closed.store(true, Ordering::Release);
        self.owner.notify_closed();
        TARGETS.with(|targets| {
            targets.borrow_mut().remove(&self.dispatch.id);
        });
    }

    #[cfg(test)]
    pub(crate) fn test_drain(&self) {
        drain_target(self.dispatch.id);
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.close();
    }
}

pub(crate) fn install(owner: &Arc<ClipboardState>) -> Registration {
    install_target(
        owner,
        #[cfg(test)]
        None,
        #[cfg(test)]
        false,
    )
}

#[cfg(test)]
pub(crate) fn install_private(
    owner: &Arc<ClipboardState>,
    pasteboard: Retained<NSPasteboard>,
) -> Registration {
    install_target(owner, Some(pasteboard), true)
}

fn install_target(
    owner: &Arc<ClipboardState>,
    #[cfg(test)] private_board: Option<Retained<NSPasteboard>>,
    #[cfg(test)] manual: bool,
) -> Registration {
    let dispatch = Arc::new(Dispatch {
        id: NEXT_TARGET.fetch_add(1, Ordering::Relaxed),
        closed: AtomicBool::new(false),
        queued: AtomicBool::new(false),
        pending: Mutex::new(None),
        #[cfg(test)]
        manual,
    });
    TARGETS.with(|targets| {
        targets.borrow_mut().insert(
            dispatch.id,
            Rc::new(Target {
                owner: Arc::downgrade(owner),
                dispatch: Arc::clone(&dispatch),
                #[cfg(test)]
                private_board,
            }),
        );
    });
    owner.attach(Arc::clone(&dispatch));
    Registration {
        dispatch,
        owner: Arc::clone(owner),
        _ui_only: std::marker::PhantomData,
    }
}

impl Dispatch {
    pub(crate) fn submit(&self, id: u64) -> Result<(), ClipboardError> {
        let mut pending = self.pending.lock().map_err(|_| ClipboardError::Scheduler)?;
        if self.closed.load(Ordering::Acquire) {
            return Err(ClipboardError::Closed);
        }
        if pending.is_some() {
            return Err(ClipboardError::Busy);
        }
        *pending = Some(id);
        drop(pending);
        if !self.queued.swap(true, Ordering::AcqRel) {
            #[cfg(test)]
            if self.manual {
                // The private-pasteboard integration test drives precisely
                // this pending queue without starting an AppKit UI loop.
                return Ok(());
            }
            let id = Box::into_raw(Box::new(self.id)).cast::<c_void>();
            // SAFETY: GCD delivers the one opaque target id on its main queue.
            unsafe {
                dispatch_async_f(std::ptr::addr_of!(_dispatch_main_q).cast_mut(), id, drain);
            }
        }
        Ok(())
    }
}

unsafe extern "C" fn drain(context: *mut c_void) {
    // SAFETY: submit transfers one boxed ID to this callback.
    let id = unsafe { Box::from_raw(context.cast::<u64>()) };
    drain_target(*id);
}

fn drain_target(id: u64) {
    let target = TARGETS.with(|targets| targets.borrow().get(&id).cloned());
    if let Some(target) = target {
        target.dispatch.queued.store(false, Ordering::Release);
        let pending = target
            .dispatch
            .pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.take());
        if let Some(id) = pending {
            if let Some(owner) = target.owner.upgrade() {
                let result = owner.payload(id).and_then(|png| {
                    // No CaptureState mutex is held while AppKit synchronously
                    // copies bytes, acknowledges, and reads back public.png.
                    #[cfg(test)]
                    if let Some(board) = &target.private_board {
                        return write_png_to_pasteboard(board, &png, || owner.begin_write(id))
                            .map(|_| ());
                    }
                    write_general_png(&owner, id, &png).map(|_| ())
                });
                owner.complete(id, result);
            }
        }
    }
}

fn write_general_png(owner: &ClipboardState, id: u64, png: &[u8]) -> Result<isize, ClipboardError> {
    let pasteboard = NSPasteboard::generalPasteboard();
    write_png_to_pasteboard(&pasteboard, png, || owner.begin_write(id))
}

fn write_png_to_pasteboard(
    pasteboard: &NSPasteboard,
    png: &[u8],
    begin_write: impl FnOnce() -> Result<(), ClipboardError>,
) -> Result<isize, ClipboardError> {
    if png.len() < 24 || !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(ClipboardError::InvalidData);
    }
    if png.len() > crate::MAX_WEB_CAPTURE_PNG_BYTES {
        return Err(ClipboardError::InvalidData);
    }
    // Capture changeCount before the bounded copy. Nothing can allocate or
    // schedule between that copy and the synchronized writing transition.
    let before = pasteboard.changeCount();
    let data = NSData::with_bytes(png);
    begin_write()?;
    // No allocation or scheduling between admission and AppKit mutation.
    let cleared = pasteboard.clearContents();
    if before.checked_add(1) != Some(cleared) {
        return Err(ClipboardError::Contended);
    }
    // SAFETY: NSPasteboardTypePNG is an immutable, public AppKit UTI symbol.
    let png_type = unsafe { NSPasteboardTypePNG };
    if !pasteboard.setData_forType(Some(&data), png_type) {
        return Err(ClipboardError::Rejected);
    }
    let committed = pasteboard.changeCount();
    if committed != cleared {
        return Err(ClipboardError::Contended);
    }
    let readback = pasteboard
        .dataForType(png_type)
        .ok_or(ClipboardError::Readback)?;
    if readback.len() != png.len()
        // SAFETY: NSData returned by AppKit is immutable and stays retained
        // until this synchronous comparison finishes.
        || unsafe { readback.as_bytes_unchecked() } != png
        || pasteboard.changeCount() != committed
    {
        return Err(ClipboardError::Readback);
    }
    Ok(committed)
}

#[cfg(test)]
pub(crate) const TEST_PNG: [u8; 70] = [
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 1, 99, 248, 63, 139, 225, 63, 0, 6,
    206, 2, 153, 89, 149, 178, 136, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    struct PrivatePasteboard(objc2::rc::Retained<NSPasteboard>);
    impl PrivatePasteboard {
        fn new() -> Self {
            Self(NSPasteboard::pasteboardWithUniqueName())
        }
    }
    impl Drop for PrivatePasteboard {
        fn drop(&mut self) {
            // SAFETY: This is AppKit's public `releaseGlobally` selector on
            // this test's unique pasteboard, never generalPasteboard.
            unsafe {
                let _: () = objc2::msg_send![&*self.0, releaseGlobally];
            }
        }
    }

    #[test]
    fn unique_native_pasteboard_acknowledges_and_reads_back_png_bytes() {
        let board = PrivatePasteboard::new();
        // Controlled 1×1 orange RGBA PNG; this unit never touches the
        // user's general pasteboard or fetches any page/personal content.
        let png = TEST_PNG;
        let committed = write_png_to_pasteboard(&board.0, &png, || Ok(())).unwrap();
        assert_eq!(board.0.changeCount(), committed);
        let invalid = write_png_to_pasteboard(&board.0, b"not png", || Ok(()));
        assert_eq!(invalid, Err(ClipboardError::InvalidData));
        assert_eq!(board.0.changeCount(), committed);
        let cancelled = write_png_to_pasteboard(&board.0, &png, || Err(ClipboardError::Cancelled));
        assert_eq!(cancelled, Err(ClipboardError::Cancelled));
        assert_eq!(board.0.changeCount(), committed);
    }

    #[test]
    fn queued_navigation_cancellation_wins_before_private_pasteboard_mutation() {
        let board = PrivatePasteboard::new();
        let png = TEST_PNG;
        let (_host, lifetime) = crate::HostLifetime::new();
        let capture = crate::capture::CaptureState::new(lifetime.clone(), 37);
        capture.test_store_retained(png.len());
        let content = capture.test_content().unwrap();
        let owner = ClipboardState::new(lifetime, Arc::clone(&capture));
        let id = owner.test_claim(content.token()).unwrap();
        let before = board.0.changeCount();
        let result = write_png_to_pasteboard(&board.0, &png, || {
            owner.navigation_changed();
            owner.begin_write(id)
        });
        assert_eq!(result, Err(ClipboardError::Cancelled));
        assert_eq!(board.0.changeCount(), before);
        owner.complete(id, Err(ClipboardError::Cancelled));

        capture.test_store_retained(png.len());
        let released = capture.test_content().unwrap();
        let id = owner.test_claim(released.token()).unwrap();
        capture.release(&released).unwrap();
        let before = board.0.changeCount();
        let result = write_png_to_pasteboard(&board.0, &png, || owner.begin_write(id));
        assert_eq!(result, Err(ClipboardError::Released));
        assert_eq!(board.0.changeCount(), before);
        owner.complete(id, Err(ClipboardError::Released));
    }
}
