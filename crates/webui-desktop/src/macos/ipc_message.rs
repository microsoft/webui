// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::rc::Rc;
use std::time::{Duration, Instant};

use block2::{DynBlock, RcBlock};
use futures_util::future::{select, Either};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{
    NSData, NSJSONReadingOptions, NSJSONSerialization, NSObject, NSObjectProtocol, NSString,
};
use objc2_web_kit::{WKScriptMessage, WKScriptMessageHandlerWithReply, WKUserContentController};

use crate::ipc::{Admission, IpcErrorCode, IpcWake};

use super::ipc::MacIpc;
use super::ipc_control::{self, bounded_string, Control, HelloControl, MAX_CONTROL_BYTES};

type Reply = DynBlock<dyn Fn(*mut AnyObject, *mut NSString)>;

define_class!(
    // SAFETY: NSObject subclass with UI-thread-only Rust state.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = Rc<MacIpc>]
    pub(super) struct DesktopIpcMessageHandler;

    // SAFETY: NSObjectProtocol has no additional requirements.
    unsafe impl NSObjectProtocol for DesktopIpcMessageHandler {}

    // SAFETY: The selector and all argument types match WebKit's with-reply protocol.
    unsafe impl WKScriptMessageHandlerWithReply for DesktopIpcMessageHandler {
        #[unsafe(method(userContentController:didReceiveScriptMessage:replyHandler:))]
        unsafe fn receive(
            &self,
            _controller: &WKUserContentController,
            message: &WKScriptMessage,
            reply: &Reply,
        ) {
            handle(self.ivars(), message, reply);
        }
    }
);

impl DesktopIpcMessageHandler {
    pub(super) fn new(mtm: MainThreadMarker, state: Rc<MacIpc>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(state);
        // SAFETY: NSObject's initializer is valid for this subclass.
        unsafe { msg_send![super(this), init] }
    }
}

fn handle(state: &Rc<MacIpc>, message: &WKScriptMessage, reply: &Reply) {
    if !trusted_message(message) {
        reject(reply);
        return;
    }
    // SAFETY: WebKit retains message and its body during this native callback.
    let body = unsafe { message.body() };
    let Some(control) = ipc_control::decode(&body) else {
        reject(reply);
        return;
    };
    match control {
        Control::Hello(hello) => hello_reply(state, hello, reply, message.mtm()),
        Control::Disconnect { generation, token } => {
            if state
                .bridge
                .disconnect_authenticated(generation, &token)
                .is_ok()
            {
                state.disconnected(generation);
            }
            reply.call((std::ptr::null_mut(), std::ptr::null_mut()));
        }
    }
}

fn trusted_message(message: &WKScriptMessage) -> bool {
    // SAFETY: Actual WebKit frame metadata, never fields supplied by JS.
    unsafe {
        let frame = message.frameInfo();
        let origin = frame.securityOrigin();
        trusted_frame(
            frame.isMainFrame(),
            bounded_string(&origin.protocol(), 16).as_deref(),
            bounded_string(&origin.host(), 16).as_deref(),
            origin.port(),
        )
    }
}

fn trusted_frame(main: bool, scheme: Option<&str>, host: Option<&str>, port: isize) -> bool {
    main && scheme == Some("webui") && host == Some("app") && port == 0
}

fn hello_reply(state: &Rc<MacIpc>, hello: HelloControl, reply: &Reply, mtm: MainThreadMarker) {
    let navigation = hello.proof.navigation;
    if !state.accepts_proof(&hello.proof) || state.hello_started.replace(true) {
        send_reply(
            reply,
            &hello,
            &Err(ipc_control::error(IpcErrorCode::PermissionDenied)),
        );
        return;
    }

    let future = state.bridge.admit(Admission {
        hello: hello.hello.clone(),
        proof: hello.proof.clone(),
    });
    let expires = Instant::now() + Duration::from_secs(5);
    let timeout = super::ipc_wake::deadline(mtm);
    let reply: RcBlock<dyn Fn(*mut AnyObject, *mut NSString)> = reply.copy();
    let weak = Rc::downgrade(state);
    let bridge = state.bridge.clone();
    let tasks = Rc::clone(&state.tasks.borrow());
    // Dropping this retained block on navigation/close rejects the old native
    // Promise automatically. No callback is ever delivered to a new document.
    let result = tasks.spawn(async move {
        let result = match select(future, timeout).await {
            Either::Left((result, _)) if Instant::now() < expires => result,
            _ => Err(ipc_control::error(IpcErrorCode::DeadlineExceeded)),
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
        if !send_reply(&reply, &hello, &result) {
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
            state.fail_document();
        }
    });
    if result.is_err() {
        state.fail_document();
    }
}

fn reject(reply: &Reply) {
    let text = NSString::from_str("Invalid desktop IPC control");
    reply.call((std::ptr::null_mut(), Retained::as_ptr(&text).cast_mut()));
}

fn send_reply(
    reply: &Reply,
    hello: &HelloControl,
    result: &Result<crate::ipc::SessionInfo, crate::ipc::IpcError>,
) -> bool {
    let Ok(bytes) = ipc_control::reply_json(hello, result) else {
        reject(reply);
        return false;
    };
    if bytes.len() > MAX_CONTROL_BYTES {
        reject(reply);
        return false;
    }
    // Only SDK-produced bounded control JSON enters Foundation serialization.
    let data = NSData::from_vec(bytes);
    match NSJSONSerialization::JSONObjectWithData_options_error(
        &data,
        NSJSONReadingOptions::empty(),
    ) {
        Ok(value) => {
            reply.call((Retained::as_ptr(&value).cast_mut(), std::ptr::null_mut()));
            true
        }
        Err(_) => {
            reject(reply);
            false
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::ipc::{DocumentActivation, Hello, IpcLimits, SessionInfo};

    #[test]
    fn actual_frame_identity_requires_main_frame_and_exact_origin() {
        assert!(trusted_frame(true, Some("webui"), Some("app"), 0));
        assert!(!trusted_frame(false, Some("webui"), Some("app"), 0));
        assert!(!trusted_frame(true, Some("https"), Some("app"), 0));
        assert!(!trusted_frame(true, Some("webui"), Some("app.evil"), 0));
        assert!(!trusted_frame(true, Some("webui"), Some("app"), 443));
        assert!(!trusted_frame(true, None, Some("app"), 0));
    }

    #[test]
    fn success_and_error_replies_echo_the_exact_document_proof_once() {
        let hello = HelloControl {
            call_id: "1".into(),
            hello: Hello {
                wire_version: 2,
                contract_name: "test".into(),
                contract_major: 1,
                schema_hash: "a".repeat(64),
            },
            proof: DocumentActivation {
                navigation: 5,
                document_nonce: [1; 16],
                challenge: [2; 16],
            },
        };
        let success = Ok(SessionInfo {
            generation: 7,
            token: "b".repeat(32),
            limits: IpcLimits::default(),
        });
        let failure = Err(ipc_control::error(IpcErrorCode::SchemaMismatch));
        for result in [success, failure] {
            let calls = Rc::new(std::cell::Cell::new(0));
            let counter = Rc::clone(&calls);
            let reply = RcBlock::new(move |value: *mut AnyObject, error: *mut NSString| {
                assert!(error.is_null());
                // SAFETY: send_reply holds the Foundation object throughout the block.
                let object = unsafe { value.as_ref() }.unwrap();
                let fields = object
                    .downcast_ref::<objc2_foundation::NSDictionary>()
                    .unwrap();
                let get = |key: &str| {
                    let value = fields.objectForKey(&NSString::from_str(key)).unwrap();
                    bounded_string(value.downcast_ref::<NSString>().unwrap(), 64).unwrap()
                };
                assert_eq!(get("callId"), "1");
                assert_eq!(get("navigation"), "5");
                assert_eq!(get("documentNonce"), "01".repeat(16));
                assert_eq!(get("challenge"), "02".repeat(16));
                counter.set(counter.get() + 1);
            });
            send_reply(&reply, &hello, &result);
            assert_eq!(calls.get(), 1);
        }
    }
}
