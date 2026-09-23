// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk4::{gio, glib};
use webkit6::{
    prelude::*, LoadEvent, UserContentInjectedFrames, UserContentManager, UserScript,
    UserScriptInjectionTime, WebView,
};

use crate::ipc::{
    CommittedMainDocument, DocumentActivation, IpcBridge, IpcError, IpcErrorCode, NativeControl,
    SessionInfo,
};
use crate::native_ipc::NativeIpcTasks;

use super::ipc_control::{error, nonce};
use super::ipc_wake::GtkWake;

pub(super) const HANDLER: &str = "webuiDesktopIpc";

pub(super) struct GtkIpc {
    pub(super) bridge: IpcBridge,
    epoch: Cell<crate::document::DocumentEpoch>,
    pub(super) proof: RefCell<Option<DocumentActivation>>,
    pub(super) session: RefCell<Option<SessionInfo>>,
    pub(super) hello_started: Cell<bool>,
    pub(super) tasks: RefCell<Rc<NativeIpcTasks>>,
    pub(super) wake: std::sync::Arc<GtkWake>,
    webview: glib::WeakRef<WebView>,
    pending_controls: RefCell<Vec<NativeControl>>,
}

impl GtkIpc {
    pub(super) fn install(
        bridge: IpcBridge,
        webview: &WebView,
        manager: &UserContentManager,
    ) -> Result<Rc<Self>, IpcError> {
        let wake = GtkWake::new();
        let state = Rc::new(Self {
            bridge,
            epoch: Cell::default(),
            proof: RefCell::new(None),
            session: RefCell::new(None),
            hello_started: Cell::new(false),
            tasks: RefCell::new(Rc::new(NativeIpcTasks::new(wake.clone(), 128))),
            wake,
            webview: webview.downgrade(),
            pending_controls: RefCell::new(Vec::new()),
        });
        state.wake.attach(&state);
        state.bridge.attach_waker(state.wake.clone())?;
        // History must create fresh document-start bootstrap state rather than
        // restoring a frozen page carrying an already-consumed capability.
        let settings =
            WebViewExt::settings(webview).ok_or_else(|| error(IpcErrorCode::Transport))?;
        settings.set_enable_page_cache(false);
        let weak = Rc::downgrade(&state);
        manager.connect_script_message_with_reply_received(
            Some(HANDLER),
            move |_, value, reply| {
                if let Some(state) = weak.upgrade() {
                    super::ipc_message::receive(&state, value, reply);
                } else {
                    reply.return_error_message("Desktop IPC frame is closed");
                }
                true
            },
        );
        if !manager.register_script_message_handler_with_reply(HANDLER, None) {
            state.close();
            return Err(error(IpcErrorCode::Transport));
        }
        manager.add_script(&UserScript::new(
            crate::ipc_assets::NATIVE_BOOTSTRAP_SCRIPT,
            UserContentInjectedFrames::TopFrame,
            UserScriptInjectionTime::Start,
            &[],
            &[],
        ));
        let weak = Rc::downgrade(&state);
        webview.connect_load_changed(move |webview, event| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            match event {
                LoadEvent::Started => state.navigation_started(),
                LoadEvent::Committed => state.committed(webview),
                _ => {}
            }
        });
        let weak = Rc::downgrade(&state);
        webview.connect_destroy(move |_| {
            if let Some(state) = weak.upgrade() {
                state.close();
            }
        });
        Ok(state)
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
        self.session.borrow_mut().take();
        self.pending_controls.borrow_mut().clear();
        self.hello_started.set(false);
        self.bridge.navigate(next);
        let previous = self
            .tasks
            .replace(Rc::new(NativeIpcTasks::new(self.wake.clone(), 128)));
        previous.close();
    }

    fn navigation_started(&self) {
        let outgoing = self.proof.borrow().clone().zip(
            self.session
                .borrow()
                .as_ref()
                .map(|session| session.generation),
        );
        if let Some((proof, generation)) =
            outgoing.filter(|(proof, _)| self.is_current(proof.navigation))
        {
            let control = NativeControl::Closed {
                generation,
                code: IpcErrorCode::Navigated,
            };
            match crate::native_ipc::control_script(&proof, control) {
                Ok(script) => {
                    if let Some(webview) = self.webview.upgrade() {
                        webview.evaluate_javascript(
                            &script,
                            None,
                            None,
                            None::<&gio::Cancellable>,
                            |_| {},
                        );
                    }
                }
                Err(error) => {
                    eprintln!("WebUI: could not encode document retirement control: {error}")
                }
            }
        }
        self.navigate();
    }

    fn committed(self: &Rc<Self>, webview: &WebView) {
        let mut epoch = self.epoch.get();
        let Some(navigation) = epoch.commit() else {
            return;
        };
        self.epoch.set(epoch);
        if !webview.uri().is_some_and(|uri| trusted_uri(uri.as_str())) {
            self.navigate();
            return;
        }
        let weak = Rc::downgrade(self);
        let view = webview.clone();
        let tasks = Rc::clone(&self.tasks.borrow());
        let result = tasks.spawn(async move {
            let probe = view.evaluate_javascript_future(
                "window.__webuiDesktopIpcV2?.documentNonce",
                None,
                None,
            );
            let timeout = glib::timeout_future(Duration::from_secs(5));
            let nonce = match futures_util::future::select(probe, timeout).await {
                futures_util::future::Either::Left((Ok(value), _)) => nonce(&value),
                _ => None,
            };
            let Some(state) = weak.upgrade().filter(|state| state.is_current(navigation)) else {
                return;
            };
            if !view.uri().is_some_and(|uri| trusted_uri(uri.as_str())) {
                state.navigate();
                return;
            }
            let Some(nonce) = nonce else {
                state.navigate();
                return;
            };
            state.activate(navigation, nonce, &view);
        });
        if result.is_err() {
            self.navigate();
        }
    }

    fn activate(self: &Rc<Self>, navigation: u64, nonce: [u8; 16], webview: &WebView) {
        let identity = CommittedMainDocument {
            navigation,
            origin: super::APP_ORIGIN.into(),
        };
        let Ok(proof) = self.bridge.begin_document(identity, nonce) else {
            self.navigate();
            return;
        };
        let Ok(script) = crate::native_ipc::activation_script(&proof) else {
            self.navigate();
            return;
        };
        *self.proof.borrow_mut() = Some(proof);
        let weak = Rc::downgrade(self);
        let view = webview.clone();
        let tasks = Rc::clone(&self.tasks.borrow());
        if tasks.spawn(async move {
            let evaluation = view.evaluate_javascript_future(&script, None, None);
            let timeout = glib::timeout_future(Duration::from_secs(5));
            let accepted = matches!(futures_util::future::select(evaluation, timeout).await,
                futures_util::future::Either::Left((Ok(value), _)) if value.is_boolean() && value.to_boolean());
            if let Some(state) = weak.upgrade().filter(|state| state.is_current(navigation)) {
                if !accepted { state.navigate(); }
            }
        }).is_err() { self.navigate(); }
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
            let pending = std::mem::take(&mut *self.pending_controls.borrow_mut());
            for control in pending {
                self.push_control(control);
            }
        }
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
        let Some(webview) = self.webview.upgrade() else {
            return;
        };
        let Ok(script) = crate::native_ipc::control_script(&proof, control) else {
            return;
        };
        webview.evaluate_javascript(&script, None, None, None::<&gio::Cancellable>, |_| {});
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
    }
}

impl Drop for GtkIpc {
    fn drop(&mut self) {
        self.close();
    }
}

pub(super) fn trusted_uri(uri: &str) -> bool {
    let Ok(uri) = glib::Uri::parse(uri, glib::UriFlags::NONE) else {
        return false;
    };
    uri.scheme().as_str() == "webui"
        && uri.host().as_deref() == Some("app")
        && uri.port() == -1
        && uri.userinfo().is_none()
}
