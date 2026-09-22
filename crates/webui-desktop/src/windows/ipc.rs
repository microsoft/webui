// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Document-scoped WebView2 control adapter. Only completion futures run on
//! this STA; application protobuf decoding and handlers belong to the core.

use anyhow::Result;
use serde_json::{Map, Value};
use std::{cell::RefCell, future::Future, rc::Rc, sync::Arc};
use webview2_com::{
    CoTaskMemPWSTR, DOMContentLoadedEventHandler, ExecuteScriptCompletedHandler,
    Microsoft::Web::WebView2::Win32::{ICoreWebView2, ICoreWebView2_2},
    NavigationStartingEventHandler,
};
use windows::{core::Interface, Win32::Foundation::HWND};

use super::{
    ipc_policy::{self, Document, MAX_TASKS},
    protocol::read_pwstr_bounded,
    wakeup::IpcWindowWake,
    webview::add_document_script,
    APP_ORIGIN,
};
use crate::{
    ipc::{CommittedMainDocument, IpcBridge, IpcError, IpcErrorCode, IpcWake, NativeControl},
    native_ipc::NativeIpcTasks,
};

/// Owned by FrameState. Event and evaluation callbacks retain only Weak refs.
pub(super) struct WindowsIpc {
    pub bridge: IpcBridge,
    pub(super) webview: ICoreWebView2,
    pub(super) hwnd: HWND,
    pub wake: Arc<IpcWindowWake>,
    pub(super) document: RefCell<Document>,
    tasks: RefCell<Rc<NativeIpcTasks>>,
    tokens: RefCell<Option<(i64, i64)>>,
    pub(super) hello_deadline: RefCell<Option<super::ipc_deadline::HelloDeadline>>,
}

/// Also close on initialization/message-loop errors after FrameState has taken
/// an Rc. This does not own or clone the frame's core IPC owner.
pub(super) struct Shutdown(pub Rc<WindowsIpc>);
impl Drop for Shutdown {
    fn drop(&mut self) {
        self.0.close();
    }
}

impl WindowsIpc {
    pub fn new(bridge: IpcBridge, webview: &ICoreWebView2, hwnd: HWND) -> Result<Rc<Self>> {
        let wake = Arc::new(IpcWindowWake::new(hwnd)?);
        let task_wake: Arc<dyn IpcWake> = wake.clone();
        Ok(Rc::new(Self {
            bridge,
            webview: webview.clone(),
            hwnd,
            wake,
            document: RefCell::new(Document::default()),
            tasks: RefCell::new(Rc::new(NativeIpcTasks::new(task_wake, MAX_TASKS))),
            tokens: RefCell::new(None),
            hello_deadline: RefCell::new(None),
        }))
    }

    /// Call only after FrameState has been installed in GWLP_USERDATA.
    pub fn install(self: &Rc<Self>, bootstrap: &str) -> Result<()> {
        if !self.bridge.is_enabled() {
            return Ok(());
        }
        add_document_script(&self.webview, bootstrap)?;
        let webview2 = self.webview.cast::<ICoreWebView2_2>()?;
        let weak = Rc::downgrade(self);
        let starting = NavigationStartingEventHandler::create(Box::new(move |_, args| {
            if let (Some(ipc), Some(args)) = (weak.upgrade(), args) {
                let mut native_id = 0;
                // SAFETY: Event args belong to this synchronous STA callback.
                unsafe {
                    args.NavigationId(&mut native_id)?;
                }
                ipc.start(native_id);
            }
            Ok(())
        }));
        let weak = Rc::downgrade(self);
        // ContentLoading precedes document-created scripts according to the
        // WebView2 contract. Use the first guaranteed post-bootstrap native
        // milestone, never a renderer "ready" message or a polling timer.
        let committed = DOMContentLoadedEventHandler::create(Box::new(move |_, args| {
            if let (Some(ipc), Some(args)) = (weak.upgrade(), args) {
                let mut native_id = 0;
                // SAFETY: Event args belong to this synchronous STA callback.
                unsafe {
                    args.NavigationId(&mut native_id)?;
                }
                ipc.commit(native_id);
            }
            Ok(())
        }));
        let mut start_token = 0;
        let mut commit_token = 0;
        // SAFETY: WebView2 retains the handlers. Their captures are weak.
        unsafe {
            self.webview
                .add_NavigationStarting(&starting, &mut start_token)?;
            if let Err(err) = webview2.add_DOMContentLoaded(&committed, &mut commit_token) {
                let _ = self.webview.remove_NavigationStarting(start_token);
                return Err(err.into());
            }
        }
        *self.tokens.borrow_mut() = Some((start_token, commit_token));
        let wake: Arc<dyn IpcWake> = self.wake.clone();
        self.bridge.attach_waker(wake)?;
        Ok(())
    }

    fn start(&self, native_id: u64) {
        let replaces_document = {
            let document = self.document.borrow();
            !document.epoch.closed && document.native_id != Some(native_id)
        };
        if replaces_document {
            self.publish_retirement(IpcErrorCode::Navigated);
        }
        let navigation = self.document.borrow_mut().start(native_id);
        if let Some(navigation) = navigation {
            self.cancel_hello_deadline();
            // Revoke before dropping native completions. Redirects do not reset
            // the document; fragment/history changes do not raise this event.
            self.bridge.navigate(navigation);
            let wake: Arc<dyn IpcWake> = self.wake.clone();
            let old = self
                .tasks
                .replace(Rc::new(NativeIpcTasks::new(wake, MAX_TASKS)));
            old.close();
        } else if self.document.borrow().epoch.closed {
            self.close();
        }
    }

    pub(super) fn trusted_source(&self) -> bool {
        // SAFETY: This is only called on the owning STA.
        read_pwstr_bounded(8192, |out| unsafe { self.webview.Source(out) })
            .is_ok_and(|source| ipc_policy::app_url(&source))
    }

    fn identity(&self, navigation: u64) -> CommittedMainDocument {
        CommittedMainDocument {
            navigation,
            origin: APP_ORIGIN.into(),
        }
    }

    fn commit(self: &Rc<Self>, native_id: u64) {
        if !self.trusted_source() {
            return;
        }
        let navigation = {
            let mut document = self.document.borrow_mut();
            if document.native_id != Some(native_id) {
                return;
            }
            let Some(navigation) = document.epoch.commit() else {
                return;
            };
            navigation
        };
        let weak = Rc::downgrade(self);
        let completion = ExecuteScriptCompletedHandler::create(Box::new(move |result, value| {
            let Some(ipc) = weak.upgrade() else {
                return Ok(());
            };
            if !ipc.current(navigation) || !ipc.trusted_source() {
                return Ok(());
            }
            let nonce = serde_json::from_str::<String>(&value)
                .ok()
                .and_then(|value| ipc_policy::nonce(&value));
            if result.is_err() || nonce.is_none() {
                ipc.transport_failed(IpcErrorCode::Transport);
                return Ok(());
            }
            if let Some(nonce) = nonce {
                match ipc.bridge.begin_document(ipc.identity(navigation), nonce) {
                    Ok(proof) => {
                        ipc.document.borrow_mut().proof = Some(proof.clone());
                        ipc.activate(proof);
                    }
                    Err(error) => ipc.transport_failed(error.code),
                }
            }
            Ok(())
        }));
        let script = CoTaskMemPWSTR::from(
            "(()=>{'use strict';if(window!==window.top||location.origin!=='https://app.webui.localhost')return null;const n=window.__webuiDesktopIpcV2?.documentNonce;return typeof n==='string'&&n.length===32?n:null;})()");
        // SAFETY: Current-main-document evaluation, never a child frame.
        if unsafe {
            self.webview
                .ExecuteScript(*script.as_ref().as_pcwstr(), &completion)
        }
        .is_err()
        {
            self.transport_failed(IpcErrorCode::Transport);
        }
    }

    fn activate(self: &Rc<Self>, proof: crate::ipc::DocumentActivation) {
        let Ok(script) = ipc_policy::activation_script(&proof) else {
            self.transport_failed(IpcErrorCode::Transport);
            return;
        };
        // The evaluated wrapper checks before invoking any document-provided
        // activate method; bootstrap performs its own independent check too.
        let script = CoTaskMemPWSTR::from(script.as_str());
        let weak = Rc::downgrade(self);
        let completion = ExecuteScriptCompletedHandler::create(Box::new(move |result, value| {
            let Some(ipc) = weak.upgrade() else {
                return Ok(());
            };
            if !ipc.document.borrow().accepts(&proof) {
                return Ok(());
            }
            if (result.is_err() || value != "true" || !ipc.trusted_source())
                && ipc.document.borrow().accepts(&proof)
            {
                ipc.transport_failed(IpcErrorCode::Transport);
            }
            Ok(())
        }));
        // SAFETY: Script is bounded native JSON and execution occurs on this STA.
        if unsafe {
            self.webview
                .ExecuteScript(*script.as_ref().as_pcwstr(), &completion)
        }
        .is_err()
        {
            self.transport_failed(IpcErrorCode::Transport);
        }
    }

    pub fn navigation(&self) -> u64 {
        self.document.borrow().epoch.navigation
    }
    pub fn current(&self, navigation: u64) -> bool {
        self.document.borrow().current(navigation)
    }

    pub fn spawn(&self, future: impl Future<Output = ()> + 'static) -> Result<(), IpcError> {
        let tasks = Rc::clone(&self.tasks.borrow());
        tasks.spawn(future)
    }

    pub fn drain(&self, cookie: usize) {
        if !self.wake.take(cookie) || self.document.borrow().epoch.closed {
            return;
        }
        let tasks = Rc::clone(&self.tasks.borrow());
        tasks.poll_ready();
        let Some(current) = self.document.borrow().generation else {
            return;
        };
        let navigation = self.navigation();
        // Core's queue is bounded. Yield to the pump if it replenishes while
        // draining; no idle polling and no unbounded UI callback loop.
        for _ in 0..64 {
            if !self.current(navigation) || self.document.borrow().generation != Some(current) {
                return;
            }
            let control = match self.bridge.take_control() {
                Ok(Some(control)) => control,
                Ok(None) => return,
                Err(_) => return,
            };
            let (generation, kind, code) = match control {
                NativeControl::Ready { generation } => (generation, "ready", None),
                NativeControl::Closed { generation, code } => (generation, "closed", Some(code)),
            };
            let mut message = Map::from_iter([
                ("kind".into(), Value::String(kind.into())),
                ("generation".into(), Value::String(generation.to_string())),
            ]);
            if let Some(code) = code {
                message.insert("code".into(), Value::String(code.as_str().into()));
            }
            if generation == current
                && self.current(navigation)
                && self.post(&Value::Object(message)).is_err()
            {
                if self.current(navigation) && self.document.borrow().generation == Some(current) {
                    self.transport_failed(IpcErrorCode::Transport);
                }
                return;
            }
        }
        let _ = self.wake.wake();
    }

    pub fn transport_failed(&self, code: IpcErrorCode) {
        self.publish_retirement(code);
        self.cancel_hello_deadline();
        let credentials = {
            let mut document = self.document.borrow_mut();
            document.epoch.committed = false;
            document.proof = None;
            document.max_frame_bytes = None;
            document.generation.take().zip(document.token.take())
        };
        if let Some((generation, token)) = credentials {
            let _ = self.bridge.disconnect_authenticated(generation, &token);
        }
    }

    pub(super) fn disconnect_document(&self) {
        self.transport_failed(IpcErrorCode::Closed);
        let tasks = Rc::clone(&self.tasks.borrow());
        tasks.close();
    }

    pub fn close(&self) {
        self.publish_retirement(IpcErrorCode::Closed);
        self.cancel_hello_deadline();
        self.wake.close();
        {
            let mut document = self.document.borrow_mut();
            document.epoch.closed = true;
            document.token = None;
            document.proof = None;
            document.max_frame_bytes = None;
        }
        self.bridge.close();
        let tasks = Rc::clone(&self.tasks.borrow());
        tasks.close();
        let tokens = self.tokens.borrow_mut().take();
        if let Some((start, commit)) = tokens {
            // SAFETY: Shutdown is on the owning STA, before controller teardown.
            unsafe {
                let _ = self.webview.remove_NavigationStarting(start);
                if let Ok(webview2) = self.webview.cast::<ICoreWebView2_2>() {
                    let _ = webview2.remove_DOMContentLoaded(commit);
                }
            }
        }
    }

    pub(super) fn cancel_hello_deadline(&self) {
        let deadline = self.hello_deadline.borrow_mut().take();
        if deadline.is_some() {
            super::ipc_deadline::cancel(self.hwnd, self.wake.cookie);
        }
    }

    pub fn expire_hello(&self, cookie: usize) {
        if cookie != self.wake.cookie
            || !self
                .hello_deadline
                .borrow()
                .as_ref()
                .is_some_and(|deadline| deadline.expired(std::time::Instant::now()))
        {
            return;
        }
        let deadline = self.hello_deadline.borrow_mut().take();
        if let Some(deadline) = deadline {
            super::ipc_deadline::cancel(self.hwnd, cookie);
            self.hello_reply(
                &deadline.hello,
                Err(ipc_policy::error(IpcErrorCode::DeadlineExceeded)),
            );
            self.transport_failed(IpcErrorCode::DeadlineExceeded);
        }
    }
}

impl Drop for WindowsIpc {
    fn drop(&mut self) {
        self.close();
    }
}
