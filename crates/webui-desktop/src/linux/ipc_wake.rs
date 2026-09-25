// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use super::ipc::GtkIpc;
use crate::ipc::{IpcError, IpcErrorCode, IpcWake};

thread_local! {
    static TARGETS: RefCell<HashMap<u64, Weak<GtkIpc>>> = RefCell::new(HashMap::new());
}
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub(super) struct GtkWake {
    id: u64,
    alive: AtomicBool,
    queued: AtomicBool,
}

impl GtkWake {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            alive: AtomicBool::new(true),
            queued: AtomicBool::new(false),
        })
    }

    pub(super) fn attach(&self, target: &Rc<GtkIpc>) {
        TARGETS.with(|targets| targets.borrow_mut().insert(self.id, Rc::downgrade(target)));
    }

    pub(super) fn close(&self) {
        self.alive.store(false, Ordering::Release);
        TARGETS.with(|targets| targets.borrow_mut().remove(&self.id));
    }
}

impl IpcWake for GtkWake {
    fn wake(&self) -> Result<(), IpcError> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(super::ipc_control::error(IpcErrorCode::Closed));
        }
        if self.queued.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let id = self.id;
        // Only the opaque id crosses threads. GTK values and future polling
        // stay in the owning default main context, never on a worker waker.
        gtk4::glib::idle_add_once(move || {
            let target = TARGETS.with(|targets| targets.borrow().get(&id).and_then(Weak::upgrade));
            if let Some(target) = target {
                target.wake.queued.store(false, Ordering::Release);
                target.drain();
            }
        });
        Ok(())
    }
}
