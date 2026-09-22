// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Bounded WebView2 JSON control messages. Application bytes never pass here.

use super::{
    ipc::WindowsIpc,
    ipc_policy::{self, Control, HelloCall, MAX_CONTROL_UNITS},
    protocol::read_pwstr_bounded,
};
use crate::{
    ipc::{Admission, IpcError, IpcErrorCode, SessionInfo},
    native_ipc::hello_reply_json,
};
use serde_json::Value;
use std::rc::Rc;
use webview2_com::{
    CoTaskMemPWSTR, Microsoft::Web::WebView2::Win32::ICoreWebView2WebMessageReceivedEventArgs,
};
use windows::{
    core::{Error as WindowsError, Result as WindowsResult},
    Win32::Foundation::E_FAIL,
};

impl WindowsIpc {
    /// ICoreWebView2.WebMessageReceived receives top-level messages only.
    /// Deliberately do not subscribe to ICoreWebView2Frame.WebMessageReceived.
    pub fn message(
        self: &Rc<Self>,
        args: &ICoreWebView2WebMessageReceivedEventArgs,
    ) -> WindowsResult<()> {
        // SAFETY: Both out-parameters are live, native-owned CoTaskMem strings.
        let source = read_pwstr_bounded(8192, |out| unsafe { args.Source(out) })?;
        if !ipc_policy::app_url(&source) || !self.trusted_source() {
            return Ok(());
        }
        let Ok(raw) = read_pwstr_bounded(MAX_CONTROL_UNITS, |out| unsafe {
            args.WebMessageAsJson(out)
        }) else {
            return Ok(());
        };
        if raw.len() > MAX_CONTROL_UNITS {
            return Ok(());
        }
        let Ok(control) = serde_json::from_str::<Control>(&raw) else {
            return Ok(());
        };
        match control {
            Control::Disconnect { generation, token } => {
                let Some(generation) = ipc_policy::decimal(&generation) else {
                    return Ok(());
                };
                if ipc_policy::nonce(&token).is_some()
                    && self.document.borrow().generation == Some(generation)
                    && self
                        .bridge
                        .disconnect_authenticated(generation, &token)
                        .is_ok()
                {
                    self.disconnect_document();
                }
            }
            control => {
                if let Some(call) = control.into_hello() {
                    self.hello(call);
                }
            }
        }
        Ok(())
    }

    fn hello(self: &Rc<Self>, call: HelloCall) {
        // A previous document's same callId never authorizes a native reply.
        // Ignore duplicate correlated hellos rather than rejecting the original
        // in-flight promise with a second response using the same callId.
        if !self.document.borrow_mut().begin_hello(&call.proof) {
            return;
        }
        match super::ipc_deadline::HelloDeadline::start(self.hwnd, self.wake.cookie, &call) {
            Ok(deadline) => *self.hello_deadline.borrow_mut() = Some(deadline),
            Err(error) => {
                self.transport_failed(error.code);
                return;
            }
        }
        let admission = Admission {
            hello: call.hello.clone(),
            proof: call.proof.clone(),
        };
        let bridge = self.bridge.clone();
        let weak = Rc::downgrade(self);
        let result = self.spawn(async move {
            let result = bridge.admit(admission).await;
            let Some(ipc) = weak.upgrade() else {
                return;
            };
            ipc.expire_hello(ipc.wake.cookie);
            if !ipc.document.borrow().accepts(&call.proof) {
                // Revoke an admission arriving after timeout without delivering
                // its token. Navigation checks inside disconnect protect newer
                // documents when this is an old worker's completion.
                if let Ok(session) = result {
                    let _ = ipc
                        .bridge
                        .disconnect_authenticated(session.generation, &session.token);
                }
                return;
            }
            ipc.cancel_hello_deadline();
            ipc.document.borrow_mut().hello_pending = false;
            ipc.hello_reply(&call, result);
        });
        if let Err(error) = result {
            self.transport_failed(error.code);
        }
    }

    pub(super) fn hello_reply(&self, call: &HelloCall, result: Result<SessionInfo, IpcError>) {
        let proof = &call.proof;
        if !self.document.borrow().accepts(proof) {
            if let Ok(session) = result {
                let _ = self
                    .bridge
                    .disconnect_authenticated(session.generation, &session.token);
            }
            return;
        }
        let response = hello_reply_json(call, &result);
        if let Ok(session) = &result {
            let mut document = self.document.borrow_mut();
            document.generation = Some(session.generation);
            document.token = Some(session.token.clone());
            document.max_frame_bytes = Some(session.limits.max_frame_bytes);
        }
        if response
            .map_err(|_| ())
            .and_then(|response| self.post_json(&response).map_err(|_| ()))
            .is_err()
        {
            // A COM delivery can reenter navigation callbacks. Retire exactly
            // the failed admission, never whichever document is current now.
            if let Ok(session) = result {
                let _ = self
                    .bridge
                    .disconnect_authenticated(session.generation, &session.token);
            }
            if self.document.borrow().accepts(proof) {
                self.transport_failed(IpcErrorCode::Transport);
            }
        }
    }

    pub(super) fn post(&self, message: &Value) -> WindowsResult<()> {
        let json = message.to_string();
        self.post_json(json.as_bytes())
    }

    fn post_json(&self, json: &[u8]) -> WindowsResult<()> {
        let json = std::str::from_utf8(json).map_err(|_| WindowsError::from(E_FAIL))?;
        let json = CoTaskMemPWSTR::from(json);
        // SAFETY: Bounded metadata only; called on the WebView2 STA.
        unsafe {
            self.webview
                .PostWebMessageAsJson(*json.as_ref().as_pcwstr())
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn success_and_failure_echo_all_proof_fields_without_payload_arrays() {
        let proof = crate::ipc::DocumentActivation {
            navigation: 9,
            document_nonce: [1; 16],
            challenge: [2; 16],
        };
        let session = SessionInfo {
            generation: 7,
            token: "03".repeat(16),
            limits: crate::ipc::IpcLimits::default(),
        };
        let error = ipc_policy::error(IpcErrorCode::DeadlineExceeded);
        let call = HelloCall {
            call_id: "1".into(),
            proof,
            hello: crate::ipc::Hello {
                wire_version: 2,
                contract_name: "test".into(),
                contract_major: 1,
                schema_hash: "0".repeat(64),
            },
        };
        for result in [Ok(session), Err(error)] {
            let response: Value =
                serde_json::from_slice(&hello_reply_json(&call, &result).unwrap()).unwrap();
            assert_eq!(response["kind"], "helloResult");
            assert_eq!(response["callId"], "1");
            assert_eq!(response["navigation"], "9");
            assert_eq!(response["documentNonce"], "01".repeat(16));
            assert_eq!(response["challenge"], "02".repeat(16));
            if result.is_ok() {
                assert_eq!(response["generation"], "7");
                assert!(response.get("error").is_none());
            } else {
                assert_eq!(response["error"]["code"], "deadline-exceeded");
                assert!(response.get("token").is_none());
            }
        }
    }
}
