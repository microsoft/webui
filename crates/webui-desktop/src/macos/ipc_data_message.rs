// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Separate, bounded WKScriptMessageHandlerWithReply for application bytes.
//! No application DTO or credential is ever submitted to the HTTP server.

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{NSDictionary, NSNumber, NSObject, NSObjectProtocol, NSString};
use objc2_web_kit::{WKScriptMessage, WKScriptMessageHandlerWithReply, WKUserContentController};
use std::rc::Rc;

use crate::ipc::native_data::{DataRequest, NativeData, CHUNK_BASE64, MAX_MESSAGE_UNITS};

use super::ipc::MacIpc;
use super::ipc_control::bounded_string;
use super::ipc_message::{native_json_reply, reject, trusted_message};

type Reply = DynBlock<dyn Fn(*mut AnyObject, *mut NSString)>;

define_class!(
    // SAFETY: NSObject subclass with main-thread-only state and no Drop.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = Rc<MacIpc>]
    pub(super) struct DesktopIpcDataHandler;

    // SAFETY: NSObjectProtocol imposes no additional requirements.
    unsafe impl NSObjectProtocol for DesktopIpcDataHandler {}
    // SAFETY: Selector and argument types match WKScriptMessageHandlerWithReply.
    unsafe impl WKScriptMessageHandlerWithReply for DesktopIpcDataHandler {
        #[unsafe(method(userContentController:didReceiveScriptMessage:replyHandler:))]
        unsafe fn receive(
            &self, _controller: &WKUserContentController,
            message: &WKScriptMessage, reply: &Reply,
        ) {
            handle(self.ivars(), message, reply);
        }
    }
);

impl DesktopIpcDataHandler {
    pub(super) fn new(mtm: MainThreadMarker, state: Rc<MacIpc>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(state);
        // SAFETY: NSObject's initializer is valid for this subclass.
        unsafe { msg_send![super(this), init] }
    }
}

fn field(fields: &NSDictionary, name: &str, max: usize) -> Option<String> {
    bounded_string(
        fields
            .objectForKey(&NSString::from_str(name))?
            .downcast_ref::<NSString>()?,
        max,
    )
}

fn number(fields: &NSDictionary, name: &str, max: usize) -> Option<usize> {
    let value = fields.objectForKey(&NSString::from_str(name))?;
    let value = value.downcast_ref::<NSNumber>()?;
    let text = bounded_string(&value.stringValue(), max)?;
    (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())).then_some(())?;
    text.parse().ok()
}

fn base64_field(fields: &NSDictionary) -> Option<String> {
    let value = fields.objectForKey(&NSString::from_str("data"))?;
    let text = value.downcast_ref::<NSString>()?;
    let length = text.length();
    if length == 0 || length > CHUNK_BASE64 {
        return None;
    }
    let mut result = String::with_capacity(length);
    for index in 0..length {
        let unit = text.characterAtIndex(index);
        let byte = u8::try_from(unit).ok()?;
        if !byte.is_ascii_alphanumeric() && !matches!(byte, b'+' | b'/' | b'=') {
            return None;
        }
        result.push(char::from(byte));
    }
    Some(result)
}

fn decode(body: &AnyObject) -> Option<DataRequest> {
    let fields = body.downcast_ref::<NSDictionary>()?;
    let operation = field(fields, "operation", 7)?;
    let send = operation == "send";
    if !(send || operation == "receive") || fields.count() != if send { 9 } else { 8 } {
        return None;
    }
    Some(DataRequest {
        kind: field(fields, "kind", 10)?,
        version: u8::try_from(number(fields, "version", 2)?).ok()?,
        call_id: field(fields, "callId", 20)?,
        generation: field(fields, "generation", 20)?,
        token: field(fields, "token", 32)?,
        operation,
        offset: number(fields, "offset", 10)?,
        total_bytes: send.then(|| number(fields, "totalBytes", 10)).flatten(),
        data: if send {
            Some(base64_field(fields)?)
        } else {
            None
        },
        max_bytes: (!send).then(|| number(fields, "maxBytes", 5)).flatten(),
    })
}

fn handle(state: &Rc<MacIpc>, message: &WKScriptMessage, reply: &Reply) {
    if !state.is_local() || !trusted_message(state, message) {
        reject(reply);
        return;
    }
    // SAFETY: WebKit retains message.body for the synchronous native callback.
    let body = unsafe { message.body() };
    let Some(request) = decode(&body) else {
        reject(reply);
        return;
    };
    let navigation = state.navigation();
    let session = state.session.borrow().clone();
    let proof = state.proof.borrow().clone();
    let Some(session) = session.filter(|_| {
        proof
            .as_ref()
            .is_some_and(|proof| state.accepts_proof(proof))
    }) else {
        reject(reply);
        return;
    };
    let reply: RcBlock<dyn Fn(*mut AnyObject, *mut NSString)> = reply.copy();
    let bridge = state.bridge.clone();
    let weak = Rc::downgrade(state);
    let cursor = Rc::clone(&state.data);
    let tasks = Rc::clone(&state.tasks.borrow());
    if tasks
        .spawn(async move {
            let response =
                NativeData::exchange(&cursor, bridge, navigation, session, request).await;
            let Some(state) = weak
                .upgrade()
                .filter(|state| state.is_current(navigation) && state.trusted_current_webview())
            else {
                reject(&reply);
                return;
            };
            state.arm_data_deadline();
            match serde_json::to_vec(&response) {
                Ok(bytes) if bytes.len() <= MAX_MESSAGE_UNITS => {
                    native_json_reply(&reply, bytes);
                }
                _ => reject(&reply),
            }
        })
        .is_err()
    {
        // The retained native reply block is dropped on task failure. WebKit
        // rejects the JS Promise instead of delivering stale data.
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use objc2_foundation::{NSData, NSJSONReadingOptions, NSJSONSerialization};

    #[test]
    fn actual_wk_object_is_bounded_before_base64_is_copied() {
        let parse = |data: String| {
            NSJSONSerialization::JSONObjectWithData_options_error(
                &NSData::from_vec(data.into_bytes()),
                NSJSONReadingOptions::empty(),
            )
            .unwrap()
        };
        let payload = |data: &str| {
            format!(
                r#"{{"kind":"ipcData","version":1,"callId":"1","generation":"1","token":"{}","operation":"send","offset":0,"totalBytes":3,"data":"{data}"}}"#,
                "a".repeat(32),
            )
        };
        assert!(decode(&parse(payload("YWJj"))).is_some());
        assert!(decode(&parse(payload(&"a".repeat(CHUNK_BASE64 + 1)))).is_none());
        assert!(decode(&parse(
            payload("YWJj").replace("\"offset\":0", "\"offset\":-1")
        ))
        .is_none());
        assert!(decode(&parse(
            payload("YWJj").replace("\"offset\":0", "\"offset\":0.5")
        ))
        .is_none());
    }
}
