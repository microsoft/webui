// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Owned-listener HTTP document IPC. WebKitGTK does not report the sending
//! frame for registered handlers: both handlers live in one named isolated
//! world, and ONLY this top-frame script may access that world. A separate
//! main-world entry relays bounded same-window messages without raw handlers.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::future::{select, Either};
use gtk4::{gio, glib};
use webkit6::{
    javascriptcore as jsc, prelude::*, LoadEvent, ScriptMessageReply, UserContentInjectedFrames,
    UserContentManager, UserScript, UserScriptInjectionTime, WebView,
};

use crate::ipc::native_data::{DataRequest, NativeData, MAX_MESSAGE_UNITS};
use crate::ipc::{
    Admission, CommittedMainDocument, DocumentActivation, IpcBridge, IpcError, IpcErrorCode,
    IpcWake, NativeControl, SessionInfo,
};
use crate::native_ipc::{NativeHello, NativeIpcRetirement, NativeIpcTasks};
use crate::{HostLifetime, LocalServerFrame, LoopbackOrigin};

use super::ipc_control::{self, Control, MAX_CONTROL_BYTES};

const WORLD: &str = "webui.local.ipc.private";
const CONTROL_HANDLER: &str = "webuiDesktopIpc";
const DATA_HANDLER: &str = "webuiDesktopIpcData";
const MAIN_ENTRY: &str = include_str!("../generated/ipc/linux-local-entry.js");
const PRIVATE_MEDIATOR: &str = include_str!("../generated/ipc/linux-local-mediator.js");

thread_local! {
    static TARGETS: RefCell<HashMap<u64, Weak<GtkLocalIpc>>> = RefCell::new(HashMap::new());
}
static NEXT_WAKE: AtomicU64 = AtomicU64::new(1);

struct LocalWake {
    id: u64,
    alive: AtomicBool,
    queued: AtomicBool,
}

impl LocalWake {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            id: NEXT_WAKE.fetch_add(1, Ordering::Relaxed),
            alive: AtomicBool::new(true),
            queued: AtomicBool::new(false),
        })
    }

    fn attach(&self, state: &Rc<GtkLocalIpc>) {
        TARGETS.with(|targets| targets.borrow_mut().insert(self.id, Rc::downgrade(state)));
    }

    fn close(&self) {
        self.alive.store(false, Ordering::Release);
        TARGETS.with(|targets| targets.borrow_mut().remove(&self.id));
    }
}

impl IpcWake for LocalWake {
    fn wake(&self) -> Result<(), IpcError> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(error(IpcErrorCode::Closed));
        }
        if self.queued.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let id = self.id;
        glib::idle_add_once(move || {
            let state = TARGETS.with(|targets| targets.borrow().get(&id).and_then(Weak::upgrade));
            if let Some(state) = state {
                state.wake.queued.store(false, Ordering::Release);
                state.drain();
            }
        });
        Ok(())
    }
}

struct PendingReply {
    reply: Option<ScriptMessageReply>,
    context: jsc::Context,
    retirement: NativeIpcRetirement,
}

impl PendingReply {
    fn send_json(&mut self, bytes: &[u8], maximum: usize) -> bool {
        if bytes.len() > maximum {
            return false;
        }
        let Ok(text) = std::str::from_utf8(bytes) else {
            return false;
        };
        let value = jsc::Value::from_json(&self.context, text);
        if !value.is_object() {
            return false;
        }
        let Some(reply) = self.reply.take() else {
            return false;
        };
        reply.return_value(&value);
        true
    }
}

impl Drop for PendingReply {
    fn drop(&mut self) {
        if let Some(reply) = self.reply.take() {
            reply.return_error_message(self.retirement.code().as_str());
        }
    }
}

pub(super) struct GtkLocalIpc {
    bridge: IpcBridge,
    origin: LoopbackOrigin,
    lifetime: HostLifetime,
    epoch: Cell<crate::document::DocumentEpoch>,
    proof: RefCell<Option<DocumentActivation>>,
    session: RefCell<Option<SessionInfo>>,
    hello_started: Cell<bool>,
    tasks: RefCell<Rc<NativeIpcTasks>>,
    wake: Arc<LocalWake>,
    data: Rc<RefCell<NativeData>>,
    data_timer: RefCell<Option<glib::SourceId>>,
    pending_controls: RefCell<Vec<NativeControl>>,
    webview: glib::WeakRef<WebView>,
}

impl GtkLocalIpc {
    /// Call before creating the WebView, so both top-frame scripts exist at
    /// document start. No default-world handler is registered.
    pub(super) fn prepare(manager: &UserContentManager) -> Result<(), IpcError> {
        if !manager.register_script_message_handler_with_reply(CONTROL_HANDLER, Some(WORLD))
            || !manager.register_script_message_handler_with_reply(DATA_HANDLER, Some(WORLD))
        {
            return Err(error(IpcErrorCode::Transport));
        }
        manager.add_script(&UserScript::for_world(
            PRIVATE_MEDIATOR,
            UserContentInjectedFrames::TopFrame,
            UserScriptInjectionTime::Start,
            WORLD,
            &[],
            &[],
        ));
        manager.add_script(&UserScript::new(
            MAIN_ENTRY,
            UserContentInjectedFrames::TopFrame,
            UserScriptInjectionTime::Start,
            &[],
            &[],
        ));
        Ok(())
    }

    pub(super) fn install(
        frame: &LocalServerFrame,
        webview: &WebView,
        manager: &UserContentManager,
    ) -> Result<Rc<Self>, IpcError> {
        let bridge = frame
            .ipc_bridge()
            .ok_or_else(|| error(IpcErrorCode::Transport))?;
        let wake = LocalWake::new();
        let state = Rc::new(Self {
            bridge,
            origin: frame.origin().clone(),
            lifetime: frame.lifetime().clone(),
            epoch: Cell::default(),
            proof: RefCell::new(None),
            session: RefCell::new(None),
            hello_started: Cell::new(false),
            tasks: RefCell::new(Rc::new(NativeIpcTasks::new(wake.clone(), 128))),
            wake,
            data: Rc::new(RefCell::new(NativeData::default())),
            data_timer: RefCell::new(None),
            pending_controls: RefCell::new(Vec::new()),
            webview: webview.downgrade(),
        });
        state.wake.attach(&state);
        state.bridge.attach_waker(state.wake.clone())?;
        let settings =
            WebViewExt::settings(webview).ok_or_else(|| error(IpcErrorCode::Transport))?;
        settings.set_enable_page_cache(false);
        let weak = Rc::downgrade(&state);
        manager.connect_script_message_with_reply_received(
            Some(CONTROL_HANDLER),
            move |_, value, reply| {
                if let Some(state) = weak.upgrade() {
                    state.receive_control(value, reply);
                } else {
                    reply.return_error_message("Local IPC closed");
                }
                true
            },
        );
        let weak = Rc::downgrade(&state);
        manager.connect_script_message_with_reply_received(
            Some(DATA_HANDLER),
            move |_, value, reply| {
                if let Some(state) = weak.upgrade() {
                    state.receive_data(value, reply);
                } else {
                    reply.return_error_message("Local IPC closed");
                }
                true
            },
        );
        let weak = Rc::downgrade(&state);
        webview.connect_load_changed(move |view, event| {
            let Some(state) = weak.upgrade() else { return };
            match event {
                LoadEvent::Started => state.navigation_started(),
                LoadEvent::Committed => state.committed(view),
                _ => {}
            }
        });
        Ok(state)
    }

    fn is_current(&self, navigation: u64) -> bool {
        self.epoch.get().current(navigation)
            && self.lifetime.is_active()
            && self.webview.upgrade().is_some_and(|view| {
                view.uri()
                    .is_some_and(|url| self.origin.allows(url.as_str()))
            })
    }

    fn accepts(&self, proof: &DocumentActivation) -> bool {
        self.is_current(proof.navigation)
            && self
                .epoch
                .get()
                .accepts(self.proof.borrow().as_ref(), proof)
    }

    fn clear_data(&self) {
        if let Some(timer) = self.data_timer.borrow_mut().take() {
            timer.remove();
        }
        self.data.borrow_mut().reset();
    }

    fn navigate(&self) {
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
        self.hello_started.set(false);
        self.pending_controls.borrow_mut().clear();
        self.clear_data();
        self.bridge.navigate(next);
        let previous = self
            .tasks
            .replace(Rc::new(NativeIpcTasks::new(self.wake.clone(), 128)));
        previous.retire(IpcErrorCode::Navigated);
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
            if let Ok(script) = crate::native_ipc::control_script(&proof, control) {
                if let Some(view) = self.webview.upgrade() {
                    evaluate(&view, &script);
                }
            }
        }
        self.navigate();
    }

    fn committed(self: &Rc<Self>, view: &WebView) {
        let mut epoch = self.epoch.get();
        let Some(navigation) = epoch.commit() else {
            return;
        };
        self.epoch.set(epoch);
        if !self.is_current(navigation) {
            self.navigate();
            return;
        }
        let weak = Rc::downgrade(self);
        let view = view.clone();
        let tasks = Rc::clone(&self.tasks.borrow());
        if tasks
            .spawn(async move {
                let probe = view.evaluate_javascript_future(
                    "(()=>{if(window.webkit?.messageHandlers?.webuiDesktopIpc||window.webkit?.messageHandlers?.webuiDesktopIpcData)return null;return window.__webuiDesktopIpcV2?.documentNonce})()",
                    None,
                    None,
                );
                let nonce = match select(probe, glib::timeout_future(Duration::from_secs(5))).await
                {
                    Either::Left((Ok(value), _)) => ipc_control::nonce(&value),
                    _ => None,
                };
                let Some(state) = weak.upgrade().filter(|state| state.is_current(navigation))
                else {
                    return;
                };
                if let Some(nonce) = nonce {
                    state.activate(navigation, nonce, &view);
                } else {
                    state.navigate();
                }
            })
            .is_err()
        {
            self.navigate();
        }
    }

    fn activate(self: &Rc<Self>, navigation: u64, nonce: [u8; 16], view: &WebView) {
        if !self.is_current(navigation) {
            return;
        }
        let identity = CommittedMainDocument {
            navigation,
            origin: self.origin.as_str().into(),
        };
        let Ok(proof) = self.bridge.begin_document(identity, nonce) else {
            self.navigate();
            return;
        };
        let Ok(script) = crate::native_ipc::activation_script_local(&proof) else {
            self.navigate();
            return;
        };
        *self.proof.borrow_mut() = Some(proof);
        let weak = Rc::downgrade(self);
        let view = view.clone();
        let tasks = Rc::clone(&self.tasks.borrow());
        if tasks
            .spawn(async move {
                let future = view.evaluate_javascript_future(&script, None, None);
                let accepted = matches!(
                    select(future, glib::timeout_future(Duration::from_secs(5))).await,
                    Either::Left((Ok(value), _)) if value.is_boolean() && value.to_boolean()
                );
                if let Some(state) = weak.upgrade().filter(|state| state.is_current(navigation)) {
                    if !accepted {
                        state.navigate();
                    }
                }
            })
            .is_err()
        {
            self.navigate();
        }
    }

    fn pending(&self, reply: &ScriptMessageReply, context: jsc::Context) -> PendingReply {
        PendingReply {
            reply: Some(reply.clone()),
            context,
            retirement: self.tasks.borrow().retirement(),
        }
    }

    fn receive_control(self: &Rc<Self>, value: &jsc::Value, reply: &ScriptMessageReply) {
        if !self.lifetime.is_active() {
            reply.return_error_message("Local IPC owner retired");
            return;
        }
        let Some(context) = value.context() else {
            reply.return_error_message("Local IPC control context unavailable");
            return;
        };
        let Some(control) = ipc_control::decode_local(value) else {
            reply.return_error_message("Invalid local IPC control");
            return;
        };
        match control {
            Control::Hello(hello) => self.admit(hello, self.pending(reply, context)),
            Control::Disconnect { generation, token } => {
                if self.session.borrow().as_ref().is_some_and(|session| {
                    self.is_current(self.epoch.get().navigation) && session.generation == generation
                }) && self
                    .bridge
                    .disconnect_authenticated(generation, &token)
                    .is_ok()
                {
                    self.disconnected(generation, IpcErrorCode::Closed);
                }
                reply.return_value(&jsc::Value::new_undefined(&context));
            }
        }
    }

    fn admit(self: &Rc<Self>, hello: NativeHello, mut reply: PendingReply) {
        if !self.accepts(&hello.proof) || self.hello_started.replace(true) {
            if let Ok(bytes) = crate::native_ipc::hello_reply_json_local(
                &hello,
                &Err(error(IpcErrorCode::PermissionDenied)),
            ) {
                reply.send_json(&bytes, MAX_CONTROL_BYTES);
            }
            return;
        }
        let navigation = hello.proof.navigation;
        let future = self.bridge.admit(Admission {
            hello: hello.hello.clone(),
            proof: hello.proof.clone(),
        });
        let expires = Instant::now() + Duration::from_secs(5);
        let weak = Rc::downgrade(self);
        let bridge = self.bridge.clone();
        let tasks = Rc::clone(&self.tasks.borrow());
        if tasks
            .spawn(async move {
                let timeout =
                    glib::timeout_future(expires.saturating_duration_since(Instant::now()));
                let result = match select(future, timeout).await {
                    Either::Left((result, _)) if Instant::now() < expires => result,
                    _ => Err(error(IpcErrorCode::DeadlineExceeded)),
                };
                let Some(state) = weak.upgrade().filter(|state| state.is_current(navigation))
                else {
                    if let Ok(session) = result {
                        let _ = bridge.disconnect_authenticated(session.generation, &session.token);
                    }
                    return;
                };
                if let Ok(session) = &result {
                    *state.session.borrow_mut() = Some(session.clone());
                }
                let sent = crate::native_ipc::hello_reply_json_local(&hello, &result)
                    .is_ok_and(|bytes| reply.send_json(&bytes, MAX_CONTROL_BYTES));
                if !sent {
                    if let Ok(session) = &result {
                        let _ = state
                            .bridge
                            .disconnect_authenticated(session.generation, &session.token);
                    }
                }
                let _ = state.wake.wake();
                if result
                    .as_ref()
                    .err()
                    .is_some_and(|error| error.code == IpcErrorCode::DeadlineExceeded)
                {
                    state.navigate();
                }
            })
            .is_err()
        {
            self.navigate();
        }
    }

    fn receive_data(self: &Rc<Self>, value: &jsc::Value, reply: &ScriptMessageReply) {
        if !self.lifetime.is_active() {
            reply.return_error_message("Local IPC owner retired");
            return;
        }
        let Some(context) = value.context() else {
            reply.return_error_message("Local IPC data context unavailable");
            return;
        };
        let Some(request) = decode_data(value) else {
            reply.return_error_message("Invalid local IPC data");
            return;
        };
        let navigation = self.epoch.get().navigation;
        let Some(session) = self.session.borrow().clone().filter(|_| {
            self.proof
                .borrow()
                .as_ref()
                .is_some_and(|proof| self.accepts(proof))
        }) else {
            reply.return_error_message("No current local IPC session");
            return;
        };
        let mut reply = self.pending(reply, context);
        let weak = Rc::downgrade(self);
        let cursor = Rc::clone(&self.data);
        let bridge = self.bridge.clone();
        let tasks = Rc::clone(&self.tasks.borrow());
        // Dropping an unaccepted task drops its reply and rejects WebKit's
        // Promise. Never retain an orphan reply after bounded task overload.
        let _ = tasks.spawn(async move {
            let response =
                NativeData::exchange(&cursor, bridge, navigation, session, request).await;
            let Some(state) = weak.upgrade().filter(|state| state.is_current(navigation)) else {
                return;
            };
            state.arm_data_deadline();
            if let Ok(bytes) = serde_json::to_vec(&response) {
                reply.send_json(&bytes, MAX_MESSAGE_UNITS);
            }
        });
    }

    fn arm_data_deadline(self: &Rc<Self>) {
        if let Some(source) = self.data_timer.borrow_mut().take() {
            source.remove();
        }
        let Some(until) = self.data.borrow().next_deadline() else {
            return;
        };
        let navigation = self.epoch.get().navigation;
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(
            until
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(1)),
            move || {
                let Some(state) = weak.upgrade().filter(|state| state.is_current(navigation))
                else {
                    return;
                };
                state.data_timer.borrow_mut().take();
                state.data.borrow_mut().expire(Instant::now());
                state.arm_data_deadline();
            },
        );
        *self.data_timer.borrow_mut() = Some(source);
    }

    fn disconnected(&self, generation: u64, code: IpcErrorCode) {
        if !self
            .session
            .borrow()
            .as_ref()
            .is_some_and(|session| session.generation == generation)
        {
            return;
        }
        self.session.borrow_mut().take();
        self.clear_data();
        self.pending_controls.borrow_mut().clear();
        let previous = self
            .tasks
            .replace(Rc::new(NativeIpcTasks::new(self.wake.clone(), 128)));
        previous.retire(code);
    }

    fn drain(&self) {
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
            for control in std::mem::take(&mut *self.pending_controls.borrow_mut()) {
                self.push_control(control);
            }
        }
    }

    fn push_control(&self, control: NativeControl) {
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
        let Some(proof) = self
            .proof
            .borrow()
            .clone()
            .filter(|proof| self.accepts(proof))
        else {
            return;
        };
        let Some(view) = self.webview.upgrade() else {
            return;
        };
        let closed = match &control {
            NativeControl::Closed { code, .. } => Some(*code),
            _ => None,
        };
        if let Ok(script) = crate::native_ipc::control_script(&proof, control) {
            evaluate(&view, &script);
        }
        if let Some(code) = closed {
            self.disconnected(generation, code);
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
        tasks.retire(IpcErrorCode::Closed);
        self.clear_data();
        self.proof.borrow_mut().take();
        self.session.borrow_mut().take();
        self.pending_controls.borrow_mut().clear();
    }
}

impl Drop for GtkLocalIpc {
    fn drop(&mut self) {
        self.close();
    }
}

fn evaluate(view: &WebView, script: &str) {
    view.evaluate_javascript(script, None, None, None::<&gio::Cancellable>, |_| {});
}

// The native callback has no frame metadata. Never allocate a Rust-owned
// untrusted string until the native JSC value's complete, flat data shape and
// 24-KiB chunk have been validated in its own context.
const BOUNDED_DATA: &str = r#"(v)=>{
 'use strict';
 if(!v||typeof v!=='object'||Array.isArray(v))return null;
 const send=v.operation==='send',receive=v.operation==='receive';
 if(!send&&!receive||v.kind!=='ipcData'||v.version!==1)return null;
 const keys=send?['kind','version','callId','generation','token','operation','offset','totalBytes','data']:
                 ['kind','version','callId','generation','token','operation','offset','maxBytes'];
 const own=Object.keys(v);if(own.length!==keys.length||own.some(k=>!keys.includes(k)))return null;
 const dec=(s,max)=>typeof s==='string'&&s.length>0&&s.length<=max&&/^[0-9]+$/.test(s)&&s[0]!=='0';
 if(!dec(v.callId,20)||!dec(v.generation,20)||
    typeof v.token!=='string'||!/^[0-9a-f]{32}$/.test(v.token)||
    !Number.isSafeInteger(v.offset)||v.offset<0||v.offset>16777216)return null;
 if(send){
  if(!Number.isSafeInteger(v.totalBytes)||v.totalBytes<1||v.totalBytes>16777216||
     typeof v.data!=='string'||v.data.length<4||v.data.length>32768||
     !/^[A-Za-z0-9+/]*={0,2}$/.test(v.data))return null;
 }else if(!Number.isSafeInteger(v.maxBytes)||v.maxBytes<1||v.maxBytes>24576)return null;
 return JSON.stringify(v);
}"#;

fn decode_data(value: &jsc::Value) -> Option<DataRequest> {
    let context = value.context()?;
    let function = context.evaluate(BOUNDED_DATA)?;
    let result = function.function_callv(std::slice::from_ref(value))?;
    if !result.is_string() {
        return None;
    }
    let text = result.to_str();
    (text.len() <= MAX_MESSAGE_UNITS)
        .then(|| serde_json::from_str::<DataRequest>(text.as_str()).ok())
        .flatten()
}

#[cold]
fn error(code: IpcErrorCode) -> IpcError {
    IpcError::new(
        code,
        "Linux local native IPC operation rejected",
        "reload the current verified HTTP document",
    )
}
