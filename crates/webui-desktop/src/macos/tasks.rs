// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::rc::{Rc, Weak};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};

use crate::native_tasks::NativeTasks;

thread_local! {
    // Main-queue routing only: weak entries never own a frame or runtime.
    static TARGETS: RefCell<HashMap<u64, Weak<MainTasks>>> = RefCell::new(HashMap::new());
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

pub(super) struct MainTasks {
    pub(super) tasks: NativeTasks,
    wake: Arc<MainWake>,
}

struct MainWake {
    id: u64,
    alive: AtomicBool,
    queued: AtomicBool,
}

impl MainTasks {
    pub(super) fn new() -> Rc<Self> {
        let wake = Arc::new(MainWake {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            alive: AtomicBool::new(true),
            queued: AtomicBool::new(false),
        });
        let task_wake = Arc::clone(&wake);
        let state = Rc::new(Self {
            tasks: NativeTasks::new(Arc::new(move || task_wake.post()), 16),
            wake,
        });
        TARGETS.with(|targets| {
            targets
                .borrow_mut()
                .insert(state.wake.id, Rc::downgrade(&state))
        });
        state
    }
}

impl MainWake {
    fn post(&self) -> bool {
        if !self.alive.load(Ordering::Acquire) {
            return false;
        }
        if self.queued.swap(true, Ordering::AcqRel) {
            return true;
        }
        let context = Box::into_raw(Box::new(self.id)).cast();
        // SAFETY: GCD consumes this owned ID exactly once on the main queue.
        unsafe {
            dispatch_async_f(
                std::ptr::addr_of!(_dispatch_main_q).cast_mut(),
                context,
                drain,
            );
        }
        true
    }
}

unsafe extern "C" fn drain(context: *mut c_void) {
    // SAFETY: `post` allocates one ID per callback; GCD invokes it exactly once.
    let id = unsafe { Box::from_raw(context.cast::<u64>()) };
    let state = TARGETS.with(|targets| targets.borrow().get(&id).and_then(Weak::upgrade));
    if let Some(state) = state {
        state.wake.queued.store(false, Ordering::Release);
        state.tasks.poll_ready();
    }
}

impl Drop for MainTasks {
    fn drop(&mut self) {
        self.wake.alive.store(false, Ordering::Release);
        TARGETS.with(|targets| targets.borrow_mut().remove(&self.wake.id));
        self.tasks.close();
    }
}
