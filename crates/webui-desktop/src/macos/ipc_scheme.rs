// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use futures_util::future::{AbortHandle, Abortable};
use objc2::runtime::ProtocolObject;
use objc2::Message;
use objc2_foundation::{NSString, NSURLRequest, NSURL};
use objc2_web_kit::WKURLSchemeTask;
use prost::Message as _;

use crate::ipc::{IpcError, IpcErrorCode, OwnedIpcHttpRequest};
use crate::{DesktopHttpMethod, DesktopProtocolResponse};

use super::ipc::MacIpc;
use super::ipc_control::{bounded_string, error};
use super::response::{send_response, send_response_cancellable};

pub(super) struct IpcScheme {
    state: Rc<MacIpc>,
    pending: RefCell<HashMap<usize, Pending>>,
}

struct Pending {
    abort: AbortHandle,
    active: Rc<Cell<bool>>,
}

struct Completion {
    owner: Weak<IpcScheme>,
    key: usize,
    active: Rc<Cell<bool>>,
}

impl Drop for Completion {
    fn drop(&mut self) {
        self.active.set(false);
        if let Some(owner) = self.owner.upgrade() {
            let pending = owner.pending.borrow_mut().remove(&self.key);
            drop(pending);
        }
    }
}

impl IpcScheme {
    pub(super) fn new(state: Rc<MacIpc>) -> Rc<Self> {
        Rc::new(Self {
            state,
            pending: RefCell::new(HashMap::new()),
        })
    }

    pub(super) fn start(self: &Rc<Self>, task: &ProtocolObject<dyn WKURLSchemeTask>) -> bool {
        // SAFETY: WebKit supplies a live task throughout startURLSchemeTask.
        let request = unsafe { task.request() };
        let Some(url) = request.URL() else {
            return false;
        };
        let Some(native_path) = url.path() else {
            return false;
        };
        if !reserved_path(&native_path) {
            return false;
        }
        let Some(path) = bounded_string(&native_path, 128) else {
            send_response(
                task,
                Some(&url),
                error_response(error(IpcErrorCode::InvalidFrame)),
            );
            return true;
        };
        if matches!(
            path.as_str(),
            "/_webui/ipc/bootstrap.js" | "/_webui/ipc/runtime.js"
        ) {
            return false;
        }
        let input = prepare(&self.state, &request, &url, path);
        match input {
            Ok(input) => self.submit(task, &url, input),
            Err(error) => send_response(task, Some(&url), error_response(error)),
        }
        true
    }

    fn submit(
        self: &Rc<Self>,
        task: &ProtocolObject<dyn WKURLSchemeTask>,
        url: &NSURL,
        input: OwnedIpcHttpRequest,
    ) {
        let navigation = input.navigation;
        let key = std::ptr::from_ref(task).cast::<()>() as usize;
        let (abort, registration) = AbortHandle::new_pair();
        let active = Rc::new(Cell::new(true));
        self.pending.borrow_mut().insert(
            key,
            Pending {
                abort,
                active: Rc::clone(&active),
            },
        );
        let completion = Completion {
            owner: Rc::downgrade(self),
            key,
            active,
        };
        let future = self.state.bridge.submit(input);
        let native_task = task.retain();
        let native_url = url.retain();
        let weak = Rc::downgrade(&self.state);
        let tasks = Rc::clone(&self.state.tasks.borrow());
        let result = tasks.spawn(async move {
            let operation = async move {
                let response = future.await.unwrap_or_else(error_response);
                let Some(state) = weak.upgrade() else {
                    return;
                };
                let live = || completion.active.get() && state.is_current(navigation);
                if live() {
                    send_response_cancellable(&native_task, Some(&native_url), response, live);
                }
                drop(completion);
            };
            let _ = Abortable::new(operation, registration).await;
        });
        if let Err(error) = result {
            // spawn drops its captured future without retaining the native task.
            send_response(task, Some(url), error_response(error));
        }
    }

    pub(super) fn stop(&self, task: &ProtocolObject<dyn WKURLSchemeTask>) {
        let key = std::ptr::from_ref(task).cast::<()>() as usize;
        let pending = self.pending.borrow_mut().remove(&key);
        if let Some(pending) = pending {
            pending.active.set(false);
            pending.abort.abort();
        }
    }
}

fn reserved_path(path: &NSString) -> bool {
    const PREFIX: &[u8] = b"/_webui/ipc";
    let length = path.length();
    if length < PREFIX.len() {
        return false;
    }
    PREFIX
        .iter()
        .enumerate()
        .all(|(index, byte)| path.characterAtIndex(index) == u16::from(*byte))
        && (length == PREFIX.len() || path.characterAtIndex(PREFIX.len()) == u16::from(b'/'))
}

fn prepare(
    state: &MacIpc,
    request: &NSURLRequest,
    url: &NSURL,
    path: String,
) -> Result<OwnedIpcHttpRequest, IpcError> {
    if state.session.borrow().is_none() {
        return Err(error(IpcErrorCode::NotReady));
    }
    if !super::ipc::trusted_url(url)
        || url.query().is_some()
        || !state.is_current(state.navigation())
    {
        return Err(error(IpcErrorCode::PermissionDenied));
    }
    let method = request
        .HTTPMethod()
        .and_then(|method| bounded_string(&method, 16))
        .ok_or_else(|| error(IpcErrorCode::InvalidFrame))?;
    let token = request
        .valueForHTTPHeaderField(&NSString::from_str("X-WebUI-Ipc-Session"))
        .and_then(|value| bounded_string(&value, 32))
        .ok_or_else(|| error(IpcErrorCode::PermissionDenied))?;
    if request.HTTPBodyStream().is_some() {
        return Err(error(IpcErrorCode::Transport));
    }
    let data = request.HTTPBody();
    let length = data.as_ref().map_or(0, |body| body.length());
    if length > state.bridge.max_request_body_bytes(&path)? {
        return Err(error(IpcErrorCode::PayloadTooLarge));
    }
    if let Some(header) = request.valueForHTTPHeaderField(&NSString::from_str("Content-Length")) {
        let declared = bounded_string(&header, 20).and_then(|text| text.parse::<usize>().ok());
        if declared != Some(length) {
            return Err(error(IpcErrorCode::InvalidFrame));
        }
    }
    let mut input_permit = state.bridge.reserve_input(length)?;
    let mut body = Vec::with_capacity(length);
    if body.capacity() > length {
        input_permit.try_grow(body.capacity() - length)?;
    }
    if let Some(data) = data {
        // SAFETY: NSData is retained and immutable throughout this bounded
        // copy, and every byte of destination capacity has been reserved.
        body.extend_from_slice(unsafe { data.as_bytes_unchecked() });
    }
    Ok(OwnedIpcHttpRequest {
        navigation: state.navigation(),
        method: DesktopHttpMethod::parse(&method),
        path,
        token,
        body,
        input_permit,
    })
}

fn error_response(error: IpcError) -> DesktopProtocolResponse {
    let status = match error.code {
        IpcErrorCode::PermissionDenied => 401,
        IpcErrorCode::PayloadTooLarge => 413,
        IpcErrorCode::Overloaded => 429,
        IpcErrorCode::Navigated | IpcErrorCode::NotReady => 409,
        IpcErrorCode::Closed | IpcErrorCode::Transport => 503,
        _ => 400,
    };
    let body = crate::ipc::wire::WireError {
        code: error.code.as_str().into(),
        message: "Native IPC request rejected".into(),
        help: "Reload the trusted document and use the generated binary transport".into(),
        application_code: String::new(),
    }
    .encode_to_vec();
    DesktopProtocolResponse::new(status, "application/x-protobuf", body)
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::ipc::{IpcHost, IpcOptions, IpcRegistry, IpcWindowOwner};
    use objc2::rc::Retained;
    use objc2::{define_class, msg_send, AnyThread, DefinedClass};
    use objc2_foundation::{
        NSData, NSError, NSMutableURLRequest, NSObject, NSObjectProtocol, NSURLResponse,
    };
    use std::sync::Arc;

    struct TaskIvars {
        request: Retained<NSURLRequest>,
        calls: RefCell<Vec<&'static str>>,
    }

    define_class!(
        // SAFETY: Test-only NSObject with ordinary owned Rust ivars.
        #[unsafe(super = NSObject)]
        #[ivars = TaskIvars]
        struct IpcTestSchemeTask;
        // SAFETY: NSObjectProtocol imposes no additional requirements.
        unsafe impl NSObjectProtocol for IpcTestSchemeTask {}
        // SAFETY: Signatures match WKURLSchemeTask; all callbacks are recorded.
        unsafe impl WKURLSchemeTask for IpcTestSchemeTask {
            #[unsafe(method_id(request))]
            fn request(&self) -> Retained<NSURLRequest> {
                self.ivars().request.clone()
            }
            #[unsafe(method(didReceiveResponse:))]
            fn response(&self, _response: &NSURLResponse) {
                self.ivars().calls.borrow_mut().push("response");
            }
            #[unsafe(method(didReceiveData:))]
            fn data(&self, _data: &NSData) {
                self.ivars().calls.borrow_mut().push("data");
            }
            #[unsafe(method(didFinish))]
            fn finish(&self) {
                self.ivars().calls.borrow_mut().push("finish");
            }
            #[unsafe(method(didFailWithError:))]
            fn error(&self, _error: &NSError) {
                self.ivars().calls.borrow_mut().push("error");
            }
        }
    );

    fn request() -> Retained<NSMutableURLRequest> {
        let url = NSURL::URLWithString(&NSString::from_str("webui://app/_webui/ipc")).unwrap();
        let request = NSMutableURLRequest::requestWithURL(&url);
        request.setHTTPMethod(&NSString::from_str("POST"));
        request.setValue_forHTTPHeaderField(
            Some(&NSString::from_str(&"a".repeat(32))),
            &NSString::from_str("X-WebUI-Ipc-Session"),
        );
        request
    }

    fn task() -> Retained<IpcTestSchemeTask> {
        let request: Retained<NSURLRequest> = request().into_super();
        let this = IpcTestSchemeTask::alloc().set_ivars(TaskIvars {
            request,
            calls: RefCell::new(Vec::new()),
        });
        // SAFETY: NSObject init matches the test subclass.
        unsafe { msg_send![super(this), init] }
    }

    fn state() -> (IpcWindowOwner, Rc<MacIpc>) {
        let owner = IpcWindowOwner::new(
            Arc::new(IpcRegistry::default()),
            IpcOptions::default(),
            IpcHost::Source {
                origin: super::super::APP_ORIGIN.into(),
            },
        )
        .unwrap();
        let state = MacIpc::new(owner.bridge());
        state.commit_for_test();
        *state.session.borrow_mut() = Some(crate::ipc::SessionInfo {
            generation: 1,
            token: "a".repeat(32),
            limits: crate::ipc::IpcLimits::default(),
        });
        (owner, state)
    }

    #[test]
    fn native_task_is_owned_until_asynchronous_response_or_stop() {
        let (_owner, state) = state();
        let scheme = IpcScheme::new(Rc::clone(&state));
        let first = task();
        assert!(scheme.start(ProtocolObject::from_ref(&*first)));
        assert!(first.ivars().calls.borrow().is_empty());
        let tasks = Rc::clone(&state.tasks.borrow());
        tasks.poll_ready();
        assert_eq!(
            *first.ivars().calls.borrow(),
            ["response", "data", "finish"]
        );
        assert!(scheme.pending.borrow().is_empty());

        let cancelled = task();
        assert!(scheme.start(ProtocolObject::from_ref(&*cancelled)));
        scheme.stop(ProtocolObject::from_ref(&*cancelled));
        tasks.poll_ready();
        assert!(cancelled.ivars().calls.borrow().is_empty());
        assert!(scheme.pending.borrow().is_empty());

        let navigated = task();
        assert!(scheme.start(ProtocolObject::from_ref(&*navigated)));
        state.navigate();
        tasks.poll_ready();
        assert!(navigated.ivars().calls.borrow().is_empty());
        assert!(scheme.pending.borrow().is_empty());
        state.close();
    }

    #[test]
    fn native_body_length_not_header_claims_controls_admission() {
        let (_owner, state) = state();
        let request = request();
        request.setHTTPBody(Some(&NSData::from_vec(vec![0; 32])));
        request.setValue_forHTTPHeaderField(
            Some(&NSString::from_str("1")),
            &NSString::from_str("Content-Length"),
        );
        let url = request.URL().unwrap();
        assert_eq!(
            prepare(&state, &request, &url, "/_webui/ipc".into())
                .err()
                .unwrap()
                .code,
            IpcErrorCode::InvalidFrame
        );
        request.setValue_forHTTPHeaderField(None, &NSString::from_str("Content-Length"));
        request.setHTTPBody(Some(&NSData::from_vec(vec![0; 1_048_577])));
        assert_eq!(
            prepare(&state, &request, &url, "/_webui/ipc".into())
                .err()
                .unwrap()
                .code,
            IpcErrorCode::PayloadTooLarge
        );
        state.close();
    }
}
