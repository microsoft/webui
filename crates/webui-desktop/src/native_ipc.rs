// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::future::Future;
use std::sync::Arc;
#[cfg(test)]
use std::{
    cell::{Cell, RefCell},
    sync::atomic::Ordering,
};

use crate::ipc::{IpcError, IpcErrorCode, IpcWake};

#[cfg(any(target_os = "linux", test))]
#[path = "native_ipc_retirement.rs"]
mod retirement;
#[cfg(any(target_os = "linux", test))]
pub(crate) use retirement::NativeIpcRetirement;

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[path = "native_ipc_reentrant_tests.rs"]
mod reentrant_tests;

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[path = "native_ipc_delivery_tests.rs"]
mod delivery_tests;

#[cfg(test)]
use crate::document::proof_matches;

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

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
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
    inner: crate::native_tasks::NativeTasks,
    #[cfg(any(target_os = "linux", test))]
    retirement: NativeIpcRetirement,
}

impl NativeIpcTasks {
    pub(crate) fn new(wake: Arc<dyn IpcWake>, max_tasks: usize) -> Self {
        Self {
            inner: crate::native_tasks::NativeTasks::new(
                Arc::new(move || wake.wake().is_ok()),
                max_tasks,
            ),
            #[cfg(any(target_os = "linux", test))]
            retirement: NativeIpcRetirement::default(),
        }
    }

    pub(crate) fn spawn(&self, future: impl Future<Output = ()> + 'static) -> Result<(), IpcError> {
        self.inner.spawn(future).map_err(|error| {
            task_error(match error {
                crate::execution::WorkError::Closed => IpcErrorCode::Closed,
                crate::execution::WorkError::Overloaded => IpcErrorCode::Overloaded,
            })
        })
    }

    pub(crate) fn poll_ready(&self) {
        self.inner.poll_ready();
    }

    pub(crate) fn close(&self) {
        #[cfg(any(target_os = "linux", test))]
        self.retirement.retire(IpcErrorCode::Closed);
        self.inner.close();
    }

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn retirement(&self) -> NativeIpcRetirement {
        self.retirement.clone()
    }

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn retire(&self, code: IpcErrorCode) {
        self.retirement.retire(code);
        self.inner.close();
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
        assert_eq!(driver.inner.count(), 0);
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
        assert_eq!(driver.inner.count(), 1);
        driver.poll_ready();
        assert_eq!(driver.inner.count(), 0);
    }
}
