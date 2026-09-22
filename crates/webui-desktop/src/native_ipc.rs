// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Wake, Waker};

use crate::ipc::{IpcError, IpcErrorCode, IpcWake};

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[path = "native_ipc_reentrant_tests.rs"]
mod reentrant_tests;

#[cfg(any(test, target_os = "macos", target_os = "linux"))]
pub(crate) fn proof_matches(
    pending: &crate::ipc::DocumentActivation,
    received: &crate::ipc::DocumentActivation,
) -> bool {
    let difference = pending
        .document_nonce
        .iter()
        .chain(&pending.challenge)
        .zip(received.document_nonce.iter().chain(&received.challenge))
        .fold(0_u8, |difference, (a, b)| difference | (a ^ b));
    pending.navigation == received.navigation && difference == 0
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
pub(crate) struct NativeHello {
    pub(crate) call_id: String,
    pub(crate) hello: crate::ipc::Hello,
    pub(crate) proof: crate::ipc::DocumentActivation,
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
pub(crate) fn hello_reply_json(
    hello: &NativeHello,
    result: &Result<crate::ipc::SessionInfo, IpcError>,
) -> Result<Vec<u8>, serde_json::Error> {
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Reply<'a> {
        kind: &'static str,
        call_id: &'a str,
        #[serde(flatten)]
        proof: &'a crate::ipc::DocumentActivation,
        #[serde(flatten, skip_serializing_if = "Option::is_none")]
        session: Option<&'a crate::ipc::SessionInfo>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<Error>,
    }
    #[derive(serde::Serialize)]
    struct Error {
        code: IpcErrorCode,
    }
    serde_json::to_vec(&Reply {
        kind: "helloResult",
        call_id: &hello.call_id,
        proof: &hello.proof,
        session: result.as_ref().ok(),
        error: result
            .as_ref()
            .err()
            .map(|error| Error { code: error.code }),
    })
}

// The wrapper must reject a replacement document before calling its activate
// property: checking only inside the genuine bootstrap would expose the proof
// to a replacement's fake implementation. Strict mode also prevents a hostile
// getter from inspecting this wrapper through Function.caller.
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
pub(crate) fn activation_script(
    proof: &crate::ipc::DocumentActivation,
) -> Result<String, serde_json::Error> {
    let proof = serde_json::to_string(proof)?;
    Ok(format!("(()=>{{'use strict';const p={proof};const b=window.__webuiDesktopIpcV2;if(!b||b.documentNonce!==p.documentNonce)return false;return b.activate(p);}})()"))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn control_script(
    proof: &crate::ipc::DocumentActivation,
    control: crate::ipc::NativeControl,
) -> Result<String, serde_json::Error> {
    #[derive(serde::Serialize)]
    struct Push {
        kind: &'static str,
        generation: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<&'static str>,
    }
    let push = match control {
        crate::ipc::NativeControl::Ready { generation } => Push {
            kind: "ready",
            generation: generation.to_string(),
            code: None,
        },
        crate::ipc::NativeControl::Closed { generation, code } => Push {
            kind: "closed",
            generation: generation.to_string(),
            code: Some(code.as_str()),
        },
    };
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut nonce = String::with_capacity(32);
    for byte in proof.document_nonce {
        nonce.push(char::from(DIGITS[usize::from(byte >> 4)]));
        nonce.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    let push = serde_json::to_string(&push)?;
    // Availability controls never carry the activation challenge or token.
    Ok(format!("(()=>{{'use strict';if(window.__webuiDesktopIpcV2?.documentNonce==='{nonce}')window.__webuiDesktopIpcReceiveV2?.({push});}})()"))
}

// This is a UI-local completion driver, not an application executor. Only
// bridge completion futures and native delivery callbacks may be submitted.
pub(crate) struct NativeIpcTasks {
    tasks: RefCell<Vec<Task>>,
    count: Cell<usize>,
    polling: Cell<bool>,
    deferred_drain: Cell<bool>,
    alive: Arc<AtomicBool>,
    wake: Arc<dyn IpcWake>,
    max_tasks: usize,
}

struct Task {
    future: Pin<Box<dyn Future<Output = ()>>>,
    wake: Arc<TaskWake>,
}

struct TaskWake {
    ready: AtomicBool,
    active: AtomicBool,
    alive: Arc<AtomicBool>,
    native: Arc<dyn IpcWake>,
}

impl Wake for TaskWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        if self.alive.load(Ordering::Acquire)
            && self.active.load(Ordering::Acquire)
            && !self.ready.swap(true, Ordering::AcqRel)
        {
            let _ = self.native.wake();
        }
    }
}

impl NativeIpcTasks {
    pub(crate) fn new(wake: Arc<dyn IpcWake>, max_tasks: usize) -> Self {
        Self {
            tasks: RefCell::new(Vec::new()),
            count: Cell::new(0),
            polling: Cell::new(false),
            deferred_drain: Cell::new(false),
            alive: Arc::new(AtomicBool::new(true)),
            wake,
            max_tasks,
        }
    }

    pub(crate) fn spawn(&self, future: impl Future<Output = ()> + 'static) -> Result<(), IpcError> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(task_error(IpcErrorCode::Closed));
        }
        if self.count.get() >= self.max_tasks {
            return Err(task_error(IpcErrorCode::Overloaded));
        }
        self.count.set(self.count.get() + 1);
        self.tasks.borrow_mut().push(Task {
            future: Box::pin(future),
            wake: Arc::new(TaskWake {
                ready: AtomicBool::new(true),
                active: AtomicBool::new(true),
                alive: Arc::clone(&self.alive),
                native: Arc::clone(&self.wake),
            }),
        });
        if let Err(error) = self.wake.wake() {
            self.close();
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn poll_ready(&self) {
        if !self.alive.load(Ordering::Acquire) {
            return;
        }
        if self.polling.replace(true) {
            self.deferred_drain.set(true);
            return;
        }
        // Move the batch out before polling: callbacks may enqueue, close, or
        // destroy native objects that reenter the driver.
        let batch = std::mem::take(&mut *self.tasks.borrow_mut());
        for mut task in batch {
            if !self.alive.load(Ordering::Acquire) {
                self.count.set(self.count.get() - 1);
                continue;
            }
            let complete = if task.wake.ready.swap(false, Ordering::AcqRel) {
                let waker = Waker::from(Arc::clone(&task.wake));
                task.future
                    .as_mut()
                    .poll(&mut Context::from_waker(&waker))
                    .is_ready()
            } else {
                false
            };
            if complete || !self.alive.load(Ordering::Acquire) {
                self.count.set(self.count.get() - 1);
                // Drop the completion outside every RefCell borrow.
            } else {
                self.tasks.borrow_mut().push(task);
            }
        }
        self.polling.set(false);
        // A nested native event loop can consume a queued wake while this
        // batch is out of the registry. Replace that wake after leaving poll,
        // rather than recursively polling or leaving new/self-woken work idle.
        if self.deferred_drain.replace(false) && self.alive.load(Ordering::Acquire) {
            let ready = self
                .tasks
                .borrow()
                .iter()
                .any(|task| task.wake.ready.load(Ordering::Acquire));
            if ready {
                if let Err(error) = self.wake.wake() {
                    eprintln!("WebUI: native IPC completion reschedule failed: {error}");
                    self.close();
                }
            }
        }
    }

    pub(crate) fn close(&self) {
        self.alive.store(false, Ordering::Release);
        self.deferred_drain.set(false);
        let tasks = std::mem::take(&mut *self.tasks.borrow_mut());
        self.count.set(self.count.get() - tasks.len());
        drop(tasks);
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        self.wake.active.store(false, Ordering::Release);
    }
}

impl Drop for NativeIpcTasks {
    fn drop(&mut self) {
        self.close();
    }
}

#[cold]
fn task_error(code: IpcErrorCode) -> IpcError {
    IpcError::new(
        code,
        "native IPC completion unavailable",
        "reload the document or reduce concurrent IPC requests",
    )
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::rc::Rc;
    use std::sync::atomic::AtomicUsize;
    use std::task::Poll;

    #[derive(Default)]
    struct Signal(AtomicUsize);
    impl IpcWake for Signal {
        fn wake(&self) -> Result<(), IpcError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    #[test]
    fn proof_comparison_rejects_every_secret_byte_and_cross_document_epoch() {
        let expected = crate::ipc::DocumentActivation {
            navigation: 7,
            document_nonce: [1; 16],
            challenge: [2; 16],
        };
        assert!(proof_matches(&expected, &expected));
        for index in 0..16 {
            let mut forged = expected.clone();
            forged.challenge[index] ^= 1;
            assert!(!proof_matches(&expected, &forged));
            let mut forged = expected.clone();
            forged.document_nonce[index] ^= 1;
            assert!(!proof_matches(&expected, &forged));
        }
        let mut other = expected.clone();
        other.navigation += 1;
        assert!(!proof_matches(&expected, &other));
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn availability_push_never_contains_the_activation_challenge() {
        let proof = crate::ipc::DocumentActivation {
            navigation: 7,
            document_nonce: [1; 16],
            challenge: [2; 16],
        };
        let script =
            control_script(&proof, crate::ipc::NativeControl::Ready { generation: 9 }).unwrap();
        assert!(script.contains(&"01".repeat(16)));
        assert!(!script.contains(&"02".repeat(16)));
        assert!(!script.contains("challenge"));
        assert!(!script.contains("token"));
        assert!(script.contains("'use strict'"));
    }

    #[test]
    fn only_ready_completions_are_polled_and_closed_wakers_are_inert() {
        let signal = Arc::new(Signal::default());
        let driver = NativeIpcTasks::new(signal.clone(), 1);
        let polls = Rc::new(Cell::new(0));
        let waker = Rc::new(RefCell::new(None));
        let (counter, saved) = (polls.clone(), waker.clone());
        driver
            .spawn(std::future::poll_fn(move |cx| {
                counter.set(counter.get() + 1);
                *saved.borrow_mut() = Some(cx.waker().clone());
                Poll::Pending
            }))
            .unwrap();
        driver.poll_ready();
        driver.poll_ready();
        assert_eq!(polls.get(), 1);
        assert_eq!(signal.0.load(Ordering::Relaxed), 1);
        assert_eq!(
            driver.spawn(async {}).unwrap_err().code,
            IpcErrorCode::Overloaded
        );
        waker.borrow().as_ref().unwrap().wake_by_ref();
        driver.poll_ready();
        assert_eq!(polls.get(), 2);
        driver.close();
        waker.borrow().as_ref().unwrap().wake_by_ref();
        driver.poll_ready();
        assert_eq!(polls.get(), 2);
        assert_eq!(signal.0.load(Ordering::Relaxed), 2);
        assert_eq!(
            driver.spawn(async {}).unwrap_err().code,
            IpcErrorCode::Closed
        );
    }

    #[test]
    fn close_during_completion_drops_remaining_batch_without_polling() {
        let driver = Rc::new(NativeIpcTasks::new(Arc::new(Signal::default()), 2));
        let weak = Rc::downgrade(&driver);
        driver
            .spawn(async move {
                weak.upgrade().unwrap().close();
            })
            .unwrap();
        let polled = Rc::new(Cell::new(false));
        let flag = polled.clone();
        driver
            .spawn(async move {
                flag.set(true);
            })
            .unwrap();
        driver.poll_ready();
        assert!(!polled.get());
        assert_eq!(driver.count.get(), 0);
    }

    #[test]
    fn completion_can_enqueue_without_borrowing_across_callback() {
        let driver = Rc::new(NativeIpcTasks::new(Arc::new(Signal::default()), 2));
        let weak = Rc::downgrade(&driver);
        driver
            .spawn(async move {
                weak.upgrade().unwrap().spawn(async {}).unwrap();
            })
            .unwrap();
        driver.poll_ready();
        assert_eq!(driver.count.get(), 1);
        driver.poll_ready();
        assert_eq!(driver.count.get(), 0);
    }
}
