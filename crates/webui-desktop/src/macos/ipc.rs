// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Weak;
use objc2::runtime::AnyObject;
use objc2::MainThreadOnly;
use objc2_foundation::{NSError, NSString, NSURL};
use objc2_web_kit::{WKContentWorld, WKWebView};

use crate::ipc::{
    CommittedMainDocument, DocumentActivation, IpcBridge, IpcErrorCode, NativeControl, SessionInfo,
};
use crate::native_ipc::NativeIpcTasks;

use super::ipc_control::{bounded_string, nonce};
use super::ipc_wake::MainWake;

pub(super) struct MacIpc {
    pub(super) bridge: IpcBridge,
    pub(super) navigation: Cell<u64>,
    pub(super) tasks: RefCell<Rc<NativeIpcTasks>>,
    pub(super) wake: std::sync::Arc<MainWake>,
    pub(super) proof: RefCell<Option<DocumentActivation>>,
    pub(super) hello_started: Cell<bool>,
    pub(super) session: RefCell<Option<SessionInfo>>,
    pending_controls: RefCell<Vec<NativeControl>>,
    webview: RefCell<Weak<WKWebView>>,
    committed: Cell<bool>,
    closed: Cell<bool>,
}

impl MacIpc {
    pub(super) fn new(bridge: IpcBridge) -> Rc<Self> {
        let wake = MainWake::new();
        let state = Rc::new(Self {
            bridge,
            navigation: Cell::new(0),
            tasks: RefCell::new(Rc::new(NativeIpcTasks::new(wake.clone(), 128))),
            wake,
            proof: RefCell::new(None),
            hello_started: Cell::new(false),
            session: RefCell::new(None),
            pending_controls: RefCell::new(Vec::new()),
            webview: RefCell::new(Weak::default()),
            committed: Cell::new(false),
            closed: Cell::new(false),
        });
        state.wake.attach(&state);
        if state.bridge.attach_waker(state.wake.clone()).is_err() {
            state.close();
        }
        state
    }

    pub(super) fn attach(&self, webview: &WKWebView) {
        *self.webview.borrow_mut() = Weak::new(webview);
    }

    pub(super) fn is_current(&self, navigation: u64) -> bool {
        !self.closed.get() && self.committed.get() && self.navigation.get() == navigation
    }

    pub(super) fn accepts_proof(&self, proof: &DocumentActivation) -> bool {
        self.is_current(proof.navigation)
            && self
                .proof
                .borrow()
                .as_ref()
                .is_some_and(|pending| crate::native_ipc::proof_matches(pending, proof))
    }

    #[cfg(test)]
    pub(super) fn commit_for_test(&self) {
        self.navigate();
        self.committed.set(true);
    }

    pub(super) fn navigate(&self) {
        if self.closed.get() {
            return;
        }
        let Some(next) = self.navigation.get().checked_add(1) else {
            self.close();
            return;
        };
        self.navigation.set(next);
        self.committed.set(false);
        self.proof.borrow_mut().take();
        self.hello_started.set(false);
        self.session.borrow_mut().take();
        self.pending_controls.borrow_mut().clear();
        self.bridge.navigate(next);
        let old = self
            .tasks
            .replace(Rc::new(NativeIpcTasks::new(self.wake.clone(), 128)));
        old.close();
    }

    pub(super) fn navigation_started(&self) {
        self.navigation_started_with(|proof, generation| {
            let control = NativeControl::Closed {
                generation,
                code: IpcErrorCode::Navigated,
            };
            let script = match crate::native_ipc::control_script(&proof, control) {
                Ok(script) => script,
                Err(error) => {
                    eprintln!("WebUI: could not encode document retirement control: {error}");
                    return;
                }
            };
            let webview = self.webview.borrow().load();
            if let Some(webview) = webview {
                evaluate(&webview, &script, None);
            }
        });
    }

    fn navigation_started_with(&self, publish: impl FnOnce(DocumentActivation, u64)) {
        let outgoing = self.proof.borrow().clone().zip(
            self.session
                .borrow()
                .as_ref()
                .map(|session| session.generation),
        );
        if let Some((proof, generation)) =
            outgoing.filter(|(proof, _)| self.is_current(proof.navigation))
        {
            // Queue the outgoing document's nonce-bound terminal control before
            // clearing its identity or cancelling native response completions.
            // A late evaluation cannot target the replacement's fresh nonce.
            publish(proof, generation);
        }
        self.navigate();
    }

    pub(super) fn committed(self: &Rc<Self>, webview: &WKWebView) {
        if self.closed.get() || self.committed.replace(true) || self.navigation.get() == 0 {
            return;
        }
        if !webview_url_is_trusted(webview) {
            self.fail_document();
            return;
        }
        let navigation = self.navigation.get();
        let weak = Rc::downgrade(self);
        let callback = RcBlock::new(move |result: *mut AnyObject, error: *mut NSError| {
            let Some(state) = weak.upgrade().filter(|state| state.is_current(navigation)) else {
                return;
            };
            if !error.is_null() {
                state.fail_document();
                return;
            }
            // SAFETY: WebKit supplies a live result for the duration of this callback.
            let value = unsafe { result.as_ref() };
            let nonce = value
                .and_then(|value| value.downcast_ref::<NSString>())
                .and_then(|text| bounded_string(text, 32))
                .and_then(|value| nonce(&value));
            let Some(nonce) = nonce else {
                state.fail_document();
                return;
            };
            state.activate(navigation, nonce);
        });
        evaluate(
            webview,
            "window.__webuiDesktopIpcV2?.documentNonce",
            Some(&callback),
        );
    }

    fn activate(self: &Rc<Self>, navigation: u64, nonce: [u8; 16]) {
        let Some(webview) = self.webview.borrow().load() else {
            return;
        };
        if !self.is_current(navigation) || !webview_url_is_trusted(&webview) {
            return;
        }
        let identity = CommittedMainDocument {
            navigation,
            origin: super::APP_ORIGIN.into(),
        };
        let Ok(proof) = self.bridge.begin_document(identity, nonce) else {
            self.fail_document();
            return;
        };
        let Ok(script) = crate::native_ipc::activation_script(&proof) else {
            self.fail_document();
            return;
        };
        *self.proof.borrow_mut() = Some(proof);
        let weak = Rc::downgrade(self);
        let callback = RcBlock::new(move |result: *mut AnyObject, error: *mut NSError| {
            let Some(state) = weak.upgrade().filter(|state| state.is_current(navigation)) else {
                return;
            };
            // SAFETY: WebKit owns the result throughout this completion callback.
            let accepted = unsafe { result.as_ref() }
                .and_then(|value| value.downcast_ref::<objc2_foundation::NSNumber>())
                .is_some_and(|value| value.boolValue());
            if !error.is_null() || !accepted {
                state.fail_document();
            }
        });
        evaluate(&webview, &script, Some(&callback));
    }

    pub(super) fn fail_document(&self) {
        // A failed/cancelled activation cannot be retried in the surviving
        // document. Revocation also cancels completions before native delivery.
        self.navigate();
    }

    pub(super) fn disconnected(&self, generation: u64) {
        if !self
            .session
            .borrow()
            .as_ref()
            .is_some_and(|session| session.generation == generation)
        {
            return;
        }
        self.session.borrow_mut().take();
        self.pending_controls.borrow_mut().clear();
        let previous = self
            .tasks
            .replace(Rc::new(NativeIpcTasks::new(self.wake.clone(), 128)));
        previous.close();
    }

    pub(super) fn drain(&self) {
        let tasks = Rc::clone(&self.tasks.borrow());
        tasks.poll_ready();
        while let Ok(Some(control)) = self.bridge.take_control() {
            if self.session.borrow().is_none() {
                let mut pending = self.pending_controls.borrow_mut();
                if pending.len() == 2 {
                    pending.remove(0);
                }
                pending.push(control);
            } else {
                self.push_control(control);
            }
        }
        if self.session.borrow().is_some() {
            let controls = std::mem::take(&mut *self.pending_controls.borrow_mut());
            for control in controls {
                self.push_control(control);
            }
        }
    }

    fn push_control(&self, control: NativeControl) {
        let closed = matches!(control, NativeControl::Closed { .. });
        let generation = match &control {
            NativeControl::Ready { generation } | NativeControl::Closed { generation, .. } => {
                *generation
            }
        };
        if !self
            .session
            .borrow()
            .as_ref()
            .is_some_and(|session| session.generation == generation)
        {
            return;
        }
        let proof = self.proof.borrow().clone();
        let Some(proof) = proof.filter(|proof| self.is_current(proof.navigation)) else {
            return;
        };
        let Some(webview) = self.webview.borrow().load() else {
            return;
        };
        let Ok(script) = crate::native_ipc::control_script(&proof, control) else {
            return;
        };
        evaluate(&webview, &script, None);
        if closed {
            self.disconnected(generation);
        }
    }

    pub(super) fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        self.wake.close();
        self.bridge.close();
        let tasks = Rc::clone(&self.tasks.borrow());
        tasks.close();
        self.proof.borrow_mut().take();
        self.session.borrow_mut().take();
        self.pending_controls.borrow_mut().clear();
    }
}

impl Drop for MacIpc {
    fn drop(&mut self) {
        self.close();
    }
}

pub(super) fn trusted_url(url: &NSURL) -> bool {
    url.scheme().and_then(|s| bounded_string(&s, 16)).as_deref() == Some("webui")
        && url.host().and_then(|s| bounded_string(&s, 16)).as_deref() == Some("app")
        && url.port().is_none()
        && url.user().is_none()
        && url.password().is_none()
}

fn webview_url_is_trusted(webview: &WKWebView) -> bool {
    // SAFETY: Only used on the owning main thread with a live WebView.
    unsafe { webview.URL() }.is_some_and(|url| trusted_url(&url))
}

fn evaluate(
    webview: &WKWebView,
    script: &str,
    completion: Option<&block2::DynBlock<dyn Fn(*mut AnyObject, *mut NSError)>>,
) {
    // SAFETY: nil frame explicitly targets the current main frame; pageWorld
    // contains the document-start bootstrap. Completion blocks are copied by WebKit.
    unsafe {
        webview.evaluateJavaScript_inFrame_inContentWorld_completionHandler(
            &NSString::from_str(script),
            None,
            &WKContentWorld::pageWorld(webview.mtm()),
            completion,
        );
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[path = "ipc_tests.rs"]
mod tests;
