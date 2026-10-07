// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use block2::RcBlock;
#[cfg(feature = "local-server")]
use objc2::rc::Retained;
use objc2::rc::Weak;
use objc2::runtime::AnyObject;
use objc2::MainThreadOnly;
#[cfg(feature = "local-server")]
use objc2_foundation::NSTimer;
use objc2_foundation::{NSError, NSString};
use objc2_web_kit::{WKContentWorld, WKWebView};

use crate::ipc::{
    CommittedMainDocument, DocumentActivation, IpcBridge, IpcErrorCode, NativeControl, SessionInfo,
};
use crate::native_ipc::NativeIpcTasks;

use super::ipc_control::{bounded_string, nonce};
use super::ipc_wake::MainWake;
pub(super) use super::navigation::trusted_app_url as trusted_url;

pub(super) struct MacIpc {
    pub(super) bridge: IpcBridge,
    epoch: Cell<crate::document::DocumentEpoch>,
    pub(super) tasks: RefCell<Rc<NativeIpcTasks>>,
    pub(super) wake: std::sync::Arc<MainWake>,
    pub(super) proof: RefCell<Option<DocumentActivation>>,
    pub(super) hello_started: Cell<bool>,
    pub(super) session: RefCell<Option<SessionInfo>>,
    pending_controls: RefCell<Vec<NativeControl>>,
    webview: RefCell<Weak<WKWebView>>,
    #[cfg(feature = "local-server")]
    pub(super) local: Option<(crate::LoopbackOrigin, crate::HostLifetime)>,
    #[cfg(feature = "local-server")]
    pub(super) data: Rc<RefCell<crate::ipc::native_data::NativeData>>,
    #[cfg(feature = "local-server")]
    data_timer: RefCell<Option<Retained<NSTimer>>>,
}

impl MacIpc {
    pub(super) fn new(bridge: IpcBridge) -> Rc<Self> {
        Self::new_with_local(bridge, None)
    }

    #[cfg(feature = "local-server")]
    pub(super) fn new_local(
        bridge: IpcBridge,
        origin: crate::LoopbackOrigin,
        lifetime: crate::HostLifetime,
    ) -> Rc<Self> {
        Self::new_with_local(bridge, Some((origin, lifetime)))
    }

    fn new_with_local(
        bridge: IpcBridge,
        #[cfg(feature = "local-server")] local: Option<(
            crate::LoopbackOrigin,
            crate::HostLifetime,
        )>,
        #[cfg(not(feature = "local-server"))] _local: Option<()>,
    ) -> Rc<Self> {
        let wake = MainWake::new();
        let state = Rc::new(Self {
            bridge,
            epoch: Cell::default(),
            tasks: RefCell::new(Rc::new(NativeIpcTasks::new(wake.clone(), 128))),
            wake,
            proof: RefCell::new(None),
            hello_started: Cell::new(false),
            session: RefCell::new(None),
            pending_controls: RefCell::new(Vec::new()),
            webview: RefCell::new(Weak::default()),
            #[cfg(feature = "local-server")]
            local,
            #[cfg(feature = "local-server")]
            data: Rc::new(RefCell::new(Default::default())),
            #[cfg(feature = "local-server")]
            data_timer: RefCell::new(None),
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
        self.epoch.get().current(navigation)
    }

    pub(super) fn navigation(&self) -> u64 {
        self.epoch.get().navigation
    }

    pub(super) fn accepts_proof(&self, proof: &DocumentActivation) -> bool {
        self.epoch
            .get()
            .accepts(self.proof.borrow().as_ref(), proof)
    }

    #[cfg(test)]
    pub(super) fn commit_for_test(&self) {
        self.navigate();
        let mut epoch = self.epoch.get();
        epoch.commit();
        self.epoch.set(epoch);
    }

    pub(super) fn navigate(&self) {
        let mut epoch = self.epoch.get();
        if epoch.closed {
            return;
        }
        let Some(next) = epoch.advance() else {
            self.close();
            return;
        };
        self.epoch.set(epoch);
        self.proof.borrow_mut().take();
        self.hello_started.set(false);
        self.session.borrow_mut().take();
        self.pending_controls.borrow_mut().clear();
        #[cfg(feature = "local-server")]
        self.clear_data();
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

    pub(super) fn retire_before_history_navigation(
        self: &Rc<Self>,
        webview: &WKWebView,
        decision: &block2::DynBlock<dyn Fn(objc2_web_kit::WKNavigationActionPolicy)>,
    ) -> bool {
        let outgoing = self.proof.borrow().clone().zip(
            self.session
                .borrow()
                .as_ref()
                .map(|session| session.generation),
        );
        let Some((proof, generation)) =
            outgoing.filter(|(proof, _)| self.is_current(proof.navigation))
        else {
            return false;
        };
        let script = match crate::native_ipc::control_script(
            &proof,
            NativeControl::Closed {
                generation,
                code: IpcErrorCode::Navigated,
            },
        ) {
            Ok(script) => script,
            Err(error) => {
                eprintln!("WebUI: could not encode history retirement control: {error}");
                return false;
            }
        };
        // History traversal can abort Fetch before didStartProvisionalNavigation
        // or pagehide. Queue the outgoing realm's terminal control and revoke
        // native ownership before yielding to its asynchronous completion.
        // WebKit may traverse only after that control has been evaluated.
        // Same-document traversals do not enter this native history policy path.
        let decision = decision.copy();
        let weak = Rc::downgrade(self);
        let retired_epoch = proof.navigation.checked_add(1);
        let callback = RcBlock::new(move |_: *mut AnyObject, error: *mut NSError| {
            let current = weak.upgrade().is_some_and(|state| {
                let epoch = state.epoch.get();
                !epoch.closed && Some(epoch.navigation) == retired_epoch
            });
            let policy = if error.is_null() && current {
                objc2_web_kit::WKNavigationActionPolicy::Allow
            } else {
                eprintln!("WebUI: could not deliver history retirement; navigation cancelled");
                objc2_web_kit::WKNavigationActionPolicy::Cancel
            };
            decision.call((policy,));
        });
        evaluate(webview, &script, Some(&callback));
        self.navigate();
        true
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
        let mut epoch = self.epoch.get();
        let Some(navigation) = epoch.commit() else {
            return;
        };
        self.epoch.set(epoch);
        if !self.trusted_webview(webview) {
            self.fail_document();
            return;
        }
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
        if !self.is_current(navigation) || !self.trusted_webview(&webview) {
            return;
        }
        let identity = CommittedMainDocument {
            navigation,
            origin: self.origin().into(),
        };
        let Ok(proof) = self.bridge.begin_document(identity, nonce) else {
            self.fail_document();
            return;
        };
        let Ok(script) = (if self.is_local() {
            crate::native_ipc::activation_script_local(&proof)
        } else {
            crate::native_ipc::activation_script(&proof)
        }) else {
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
        #[cfg(feature = "local-server")]
        self.clear_data();
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
        let mut epoch = self.epoch.get();
        if epoch.closed {
            return;
        }
        epoch.closed = true;
        self.epoch.set(epoch);
        self.wake.close();
        self.bridge.close();
        let tasks = Rc::clone(&self.tasks.borrow());
        tasks.close();
        self.proof.borrow_mut().take();
        self.session.borrow_mut().take();
        self.pending_controls.borrow_mut().clear();
        #[cfg(feature = "local-server")]
        self.clear_data();
    }

    pub(super) fn is_local(&self) -> bool {
        #[cfg(feature = "local-server")]
        {
            self.local.is_some()
        }
        #[cfg(not(feature = "local-server"))]
        {
            false
        }
    }

    pub(super) fn origin(&self) -> &str {
        #[cfg(feature = "local-server")]
        if let Some((origin, _)) = &self.local {
            return origin.as_str();
        }
        super::APP_ORIGIN
    }

    pub(super) fn trusted_webview(&self, webview: &WKWebView) -> bool {
        // SAFETY: Current URL is read on the owning WebKit main thread.
        let Some(url) = (unsafe { webview.URL() }) else {
            return false;
        };
        #[cfg(feature = "local-server")]
        if let Some((origin, lifetime)) = &self.local {
            return lifetime.is_active()
                && url
                    .absoluteString()
                    .is_some_and(|value| origin.allows(&value.to_string()))
                && url.user().is_none()
                && url.password().is_none();
        }

        trusted_url(&url)
    }

    pub(super) fn trusted_current_webview(&self) -> bool {
        self.webview
            .borrow()
            .load()
            .is_some_and(|view| self.trusted_webview(&view))
    }

    pub(super) fn trusted_frame(
        &self,
        scheme: Option<&str>,
        host: Option<&str>,
        port: isize,
    ) -> bool {
        #[cfg(feature = "local-server")]
        if let Some((origin, lifetime)) = &self.local {
            return lifetime.is_active()
                && scheme.zip(host).is_some_and(|(scheme, host)| {
                    origin.matches_security_origin(scheme, host, port)
                });
        }
        scheme == Some("webui") && host == Some("app") && port == 0
    }

    #[cfg(feature = "local-server")]
    fn clear_data(&self) {
        if let Some(timer) = self.data_timer.borrow_mut().take() {
            timer.invalidate();
        }
        self.data.borrow_mut().reset();
    }

    /// At most one native run-loop timer per local document, rearmed only when
    /// a cursor is retained. Invalidation on navigation prevents stale timers
    /// from touching a replacement document.
    #[cfg(feature = "local-server")]
    pub(super) fn arm_data_deadline(self: &Rc<Self>) {
        if let Some(timer) = self.data_timer.borrow_mut().take() {
            timer.invalidate();
        }
        let Some(until) = self.data.borrow().next_deadline() else {
            return;
        };
        let interval = until
            .saturating_duration_since(std::time::Instant::now())
            .as_secs_f64()
            .max(0.001);
        let navigation = self.navigation();
        let weak = Rc::downgrade(self);
        let block = RcBlock::new(move |_: std::ptr::NonNull<NSTimer>| {
            let Some(state) = weak.upgrade().filter(|state| state.is_current(navigation)) else {
                return;
            };
            state.data.borrow_mut().expire(std::time::Instant::now());
            state.arm_data_deadline();
        });
        // SAFETY: This callback and its timer are created/invalidated only on
        // WebKit's main run loop; it captures a weak document owner.
        let timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_repeats_block(interval, false, &block)
        };
        *self.data_timer.borrow_mut() = Some(timer);
    }
}

impl Drop for MacIpc {
    fn drop(&mut self) {
        self.close();
    }
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
