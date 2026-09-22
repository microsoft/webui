// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::rc::Rc;
use std::time::{Duration, Instant};

use futures_util::future::{select, Either};
use gtk4::glib;
use webkit6::{javascriptcore as jsc, ScriptMessageReply};

use crate::ipc::{Admission, IpcError, IpcErrorCode, IpcWake, SessionInfo};
use crate::native_ipc::{hello_reply_json, NativeHello};

use super::ipc::GtkIpc;
use super::ipc_control::{decode, error, Control, MAX_CONTROL_BYTES};

// ScriptMessageReply::clone refs the native reply. Its sole completion owner
// lives in the UI-local task and settles it on timeout, teardown or success.
struct PendingReply {
    reply: Option<ScriptMessageReply>,
    context: jsc::Context,
}

impl PendingReply {
    fn send(&mut self, hello: &NativeHello, result: &Result<SessionInfo, IpcError>) -> bool {
        let Ok(bytes) = hello_reply_json(hello, result) else {
            return false;
        };
        if bytes.len() > MAX_CONTROL_BYTES {
            return false;
        }
        let Ok(json) = std::str::from_utf8(&bytes) else {
            return false;
        };
        let value = jsc::Value::from_json(&self.context, json);
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
            reply.return_error_message("Desktop IPC admission cancelled or unavailable");
        }
    }
}

pub(super) fn receive(state: &Rc<GtkIpc>, value: &jsc::Value, reply: &ScriptMessageReply) {
    let Some(context) = value.context() else {
        reply.return_error_message("Desktop IPC control context unavailable");
        return;
    };
    let Some(control) = decode(value) else {
        reply.return_error_message("Invalid desktop IPC control");
        return;
    };
    match control {
        Control::Disconnect { generation, token } => {
            // No callback-origin claim: possession of the document's secret
            // authenticates the revision-four main-document capability.
            if state
                .bridge
                .disconnect_authenticated(generation, &token)
                .is_ok()
            {
                state.disconnected(generation);
            }
            reply.return_value(&jsc::Value::new_undefined(&context));
        }
        Control::Hello(hello) => {
            let pending = PendingReply {
                reply: Some(reply.clone()),
                context,
            };
            admit(state, hello, pending);
        }
    }
}

fn admit(state: &Rc<GtkIpc>, hello: NativeHello, mut reply: PendingReply) {
    if !state.accepts_proof(&hello.proof) || state.hello_started.replace(true) {
        reply.send(&hello, &Err(error(IpcErrorCode::PermissionDenied)));
        return;
    }
    let navigation = hello.proof.navigation;
    let future = state.bridge.admit(Admission {
        hello: hello.hello.clone(),
        proof: hello.proof.clone(),
    });
    let expires = Instant::now() + Duration::from_secs(5);
    let weak = Rc::downgrade(state);
    let bridge = state.bridge.clone();
    let tasks = Rc::clone(&state.tasks.borrow());
    if tasks
        .spawn(async move {
            let timeout = glib::timeout_future(expires.saturating_duration_since(Instant::now()));
            let result = match select(future, timeout).await {
                Either::Left((result, _)) if Instant::now() < expires => result,
                _ => Err(error(IpcErrorCode::DeadlineExceeded)),
            };
            let Some(state) = weak.upgrade().filter(|state| state.is_current(navigation)) else {
                if let Ok(session) = result {
                    let _ = bridge.disconnect_authenticated(session.generation, &session.token);
                }
                return;
            };
            if let Ok(session) = &result {
                *state.session.borrow_mut() = Some(session.clone());
            }
            if !reply.send(&hello, &result) {
                if let Ok(session) = &result {
                    let _ = bridge.disconnect_authenticated(session.generation, &session.token);
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
        state.navigate();
    }
}
