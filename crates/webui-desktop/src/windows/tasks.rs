// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::native_tasks::NativeTasks;
use std::rc::Rc;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::PostMessageW,
};

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

pub(super) struct ApplicationTasks {
    pub(super) tasks: NativeTasks,
    target: Arc<Mutex<Option<usize>>>,
    id: usize,
}

impl ApplicationTasks {
    pub(super) fn new(hwnd: HWND) -> Rc<Self> {
        let target = Arc::new(Mutex::new(Some(hwnd.0 as usize)));
        let wake_target = Arc::clone(&target);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        Rc::new(Self {
            tasks: NativeTasks::new(
                Arc::new(move || {
                    let Ok(target) = wake_target.lock() else {
                        return false;
                    };
                    let Some(handle) = *target else {
                        return false;
                    };
                    // SAFETY: PostMessage is thread-safe; close serializes with this
                    // lock, and an ID rejects messages after HWND reuse.
                    unsafe {
                        PostMessageW(
                            Some(HWND(handle as *mut _)),
                            super::APP_WAKE_MESSAGE,
                            WPARAM(id),
                            LPARAM(0),
                        )
                        .is_ok()
                    }
                }),
                16,
            ),
            target,
            id,
        })
    }

    pub(super) fn drain(&self, id: usize) {
        if id == self.id {
            self.tasks.poll_ready();
        }
    }
}

impl Drop for ApplicationTasks {
    fn drop(&mut self) {
        if let Ok(mut target) = self.target.lock() {
            target.take();
        }
        self.tasks.close();
    }
}
