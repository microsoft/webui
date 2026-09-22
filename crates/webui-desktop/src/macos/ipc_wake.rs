// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::future::Future;
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_foundation::NSTimer;

use crate::ipc::{IpcError, IpcErrorCode, IpcWake};

use super::ipc::MacIpc;

thread_local! {
    static TARGETS: RefCell<HashMap<u64, Weak<MacIpc>>> = RefCell::new(HashMap::new());
}
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[link(name = "System")]
unsafe extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_async_f(
        queue: *mut c_void,
        context: *mut c_void,
        work: unsafe extern "C" fn(*mut c_void),
    );
}

pub(super) struct MainWake {
    id: u64,
    alive: AtomicBool,
    queued: AtomicBool,
}

impl MainWake {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            alive: AtomicBool::new(true),
            queued: AtomicBool::new(false),
        })
    }

    pub(super) fn attach(&self, target: &Rc<MacIpc>) {
        TARGETS.with(|targets| targets.borrow_mut().insert(self.id, Rc::downgrade(target)));
    }

    pub(super) fn close(&self) {
        self.alive.store(false, Ordering::Release);
        TARGETS.with(|targets| targets.borrow_mut().remove(&self.id));
    }
}

impl IpcWake for MainWake {
    fn wake(&self) -> Result<(), IpcError> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(super::ipc_control::error(IpcErrorCode::Closed));
        }
        if self.queued.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let context = Box::into_raw(Box::new(self.id)).cast();
        // SAFETY: GCD takes the owned context exactly once on the main queue.
        unsafe {
            dispatch_async_f(
                std::ptr::addr_of!(_dispatch_main_q).cast_mut(),
                context,
                drain,
            )
        };
        Ok(())
    }
}

unsafe extern "C" fn drain(context: *mut c_void) {
    // SAFETY: Allocated by wake and consumed exactly once by GCD.
    let id = unsafe { Box::from_raw(context.cast::<u64>()) };
    let target = TARGETS.with(|targets| targets.borrow().get(&id).and_then(Weak::upgrade));
    if let Some(target) = target {
        target.wake.queued.store(false, Ordering::Release);
        target.drain();
    }
}

pub(super) struct Deadline {
    receiver: futures_channel::oneshot::Receiver<()>,
    timer: Retained<NSTimer>,
}

impl Future for Deadline {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        Pin::new(&mut self.receiver).poll(cx).map(|_| ())
    }
}

impl Drop for Deadline {
    fn drop(&mut self) {
        // The driver and its futures are UI-local, so timer invalidation is on
        // the run loop that created it, including navigation/close cancellation.
        self.timer.invalidate();
    }
}

pub(super) fn deadline(_mtm: MainThreadMarker) -> Deadline {
    let (sender, receiver) = futures_channel::oneshot::channel();
    let sender = std::sync::Mutex::new(Some(sender));
    let block = RcBlock::new(move |_timer: std::ptr::NonNull<NSTimer>| {
        let sender = sender.lock().ok().and_then(|mut slot| slot.take());
        if let Some(sender) = sender {
            let _ = sender.send(());
        }
    });
    // SAFETY: The block captures only a Send + Sync sender mutex; the one-shot
    // timer is scheduled and later invalidated on this proven main thread.
    let timer =
        unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(5.0, false, &block) };
    Deadline { receiver, timer }
}
