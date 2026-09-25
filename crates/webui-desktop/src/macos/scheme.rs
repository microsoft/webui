// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Custom `webui://` URL-scheme handler that serves the runtime's protocol
//! responses directly to `WKWebView`. Each handler owns its frame's runtime.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use crate::{
    DesktopHttpMethod, DesktopProtocolRequest, DesktopProtocolResponse, DesktopRuntime,
    DEFAULT_MAX_REQUEST_BYTES,
};
use objc2::rc::autoreleasepool;
use objc2::runtime::ProtocolObject;
use objc2::DefinedClass;
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly, Message};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString, NSURLRequest, NSURL};
use objc2_web_kit::{WKURLSchemeHandler, WKURLSchemeTask, WKWebView};

use super::response::send_response;
use crate::execution::ApplicationExecutor;

pub(super) struct SchemeHandlerIvars {
    runtime: Arc<DesktopRuntime>,
    executor: Arc<ApplicationExecutor>,
    tasks: Rc<super::tasks::MainTasks>,
    pending: Rc<RefCell<HashMap<usize, Arc<AtomicBool>>>>,
    #[cfg(feature = "application-ipc")]
    state: Option<std::rc::Rc<super::ipc::MacIpc>>,
    #[cfg(feature = "application-ipc")]
    ipc: Option<std::rc::Rc<super::ipc_scheme::IpcScheme>>,
}

define_class!(
    // SAFETY: Scheme handler is an NSObject subclass with no Drop implementation.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = SchemeHandlerIvars]
    pub(super) struct DesktopSchemeHandler;

    // SAFETY: NSObjectProtocol has no additional safety requirements.
    unsafe impl NSObjectProtocol for DesktopSchemeHandler {}

    // SAFETY: Method signatures match WKURLSchemeHandler.
    #[allow(non_snake_case)]
    unsafe impl WKURLSchemeHandler for DesktopSchemeHandler {
        #[unsafe(method(webView:startURLSchemeTask:))]
        unsafe fn webView_startURLSchemeTask(
            &self,
            _web_view: &WKWebView,
            url_scheme_task: &ProtocolObject<dyn WKURLSchemeTask>,
        ) {
            autoreleasepool(|_| {
                #[cfg(feature = "application-ipc")]
                if self
                    .ivars()
                    .ipc
                    .as_ref()
                    .is_some_and(|ipc| ipc.start(url_scheme_task))
                {
                    return;
                }
                handle_scheme_task(url_scheme_task, self.ivars());
            });
        }

        #[unsafe(method(webView:stopURLSchemeTask:))]
        unsafe fn webView_stopURLSchemeTask(
            &self,
            _web_view: &WKWebView,
            _url_scheme_task: &ProtocolObject<dyn WKURLSchemeTask>,
        ) {
            let key = task_key(_url_scheme_task);
            if let Some(live) = self.ivars().pending.borrow_mut().remove(&key) {
                live.store(false, Ordering::Release);
            }
            #[cfg(feature = "application-ipc")]
            if let Some(ipc) = &self.ivars().ipc {
                ipc.stop(_url_scheme_task);
            }
        }
    }
);

impl DesktopSchemeHandler {
    pub(super) fn new(
        mtm: MainThreadMarker,
        runtime: Arc<DesktopRuntime>,
        executor: Arc<ApplicationExecutor>,
        #[cfg(feature = "application-ipc")] state: Option<std::rc::Rc<super::ipc::MacIpc>>,
    ) -> objc2::rc::Retained<Self> {
        #[cfg(feature = "application-ipc")]
        let ipc = state
            .as_ref()
            .map(|state| super::ipc_scheme::IpcScheme::new(std::rc::Rc::clone(state)));
        let this = Self::alloc(mtm).set_ivars(SchemeHandlerIvars {
            runtime,
            executor,
            tasks: super::tasks::MainTasks::new(),
            pending: Rc::default(),
            #[cfg(feature = "application-ipc")]
            state,
            #[cfg(feature = "application-ipc")]
            ipc,
        });
        // SAFETY: NSObject init has the expected signature for this subclass.
        unsafe { msg_send![super(this), init] }
    }

    #[cfg(feature = "application-ipc")]
    pub(super) fn ipc_state(&self) -> Option<std::rc::Rc<super::ipc::MacIpc>> {
        self.ivars().state.clone()
    }
}

fn task_key(task: &ProtocolObject<dyn WKURLSchemeTask>) -> usize {
    std::ptr::from_ref(task).cast::<()>() as usize
}

fn handle_scheme_task(task: &ProtocolObject<dyn WKURLSchemeTask>, state: &SchemeHandlerIvars) {
    // SAFETY: WebKit invokes this method with a live task object.
    let request = unsafe { task.request() };
    let Some(url) = request.URL() else {
        send_response(
            task,
            None,
            DesktopProtocolResponse::text(400, "Bad Request"),
        );
        return;
    };
    let path = request_path(&url);
    if !super::navigation::trusted_app_url(&url) {
        send_response(
            task,
            Some(&url),
            DesktopProtocolResponse::text(403, "Untrusted desktop origin"),
        );
        return;
    }
    if state.pending.borrow().len() >= 16 || path.len() > 8192 {
        send_response(
            task,
            Some(&url),
            DesktopProtocolResponse::text(503, "Desktop request capacity exceeded"),
        );
        return;
    }
    let method = request
        .HTTPMethod()
        .map(|method| DesktopHttpMethod::parse(&method.to_string()))
        .unwrap_or(DesktopHttpMethod::Get);
    let accept = request
        .allHTTPHeaderFields()
        .and_then(|headers| headers.objectForKey(&NSString::from_str("Accept")))
        .map(|value| value.to_string())
        .unwrap_or_default();
    let body = match request_body(&request) {
        Ok(body) => body,
        Err(response) => {
            send_response(task, Some(&url), response);
            return;
        }
    };
    let wants_json = accept.contains("json") || accept.contains("ndjson");

    let runtime = Arc::clone(&state.runtime);
    let live = Arc::new(AtomicBool::new(true));
    let worker_live = Arc::clone(&live);
    let work = state.executor.submit(move || {
        if !worker_live.load(Ordering::Acquire) {
            return DesktopProtocolResponse::text(499, "Desktop request cancelled");
        }
        runtime
            .handle_request(&DesktopProtocolRequest {
                method,
                path: &path,
                body: &body,
                wants_json,
            })
            .unwrap_or_else(|err| DesktopProtocolResponse::text(500, err.chain_message()))
    });
    let Ok(work) = work else {
        send_response(
            task,
            Some(&url),
            DesktopProtocolResponse::text(503, "Desktop application executor is unavailable"),
        );
        return;
    };
    let key = task_key(task);
    state.pending.borrow_mut().insert(key, Arc::clone(&live));
    let pending = Rc::clone(&state.pending);
    let owned_task = task.retain();
    let executor = Arc::clone(&state.executor);
    if state
        .tasks
        .tasks
        .spawn(async move {
            let response = work.await.unwrap_or_else(|_| {
                DesktopProtocolResponse::text(503, "Desktop application executor closed")
            });
            if live.load(Ordering::Acquire) {
                super::response::send_application_response(
                    &owned_task,
                    &url,
                    response,
                    &executor,
                    || live.load(Ordering::Acquire),
                )
                .await;
            }
            pending.borrow_mut().remove(&key);
        })
        .is_err()
    {
        state.pending.borrow_mut().remove(&key);
        send_response(
            task,
            None,
            DesktopProtocolResponse::text(503, "Desktop completion capacity exceeded"),
        );
    }
}

fn request_path(url: &NSURL) -> String {
    let mut path = url
        .path()
        .map(|path| path.to_string())
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| "/".to_string());
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(&query.to_string());
    }
    path
}

fn request_body(request: &NSURLRequest) -> std::result::Result<Vec<u8>, DesktopProtocolResponse> {
    let Some(body) = request.HTTPBody() else {
        return Ok(Vec::new());
    };
    let len = body.length();
    if len == 0 {
        return Ok(Vec::new());
    }
    if len > usize::try_from(DEFAULT_MAX_REQUEST_BYTES).unwrap_or(usize::MAX) {
        return Err(DesktopProtocolResponse::text(
            413,
            "desktop request body exceeds the configured size limit",
        ));
    }
    // SAFETY: NSData's `bytes` pointer is valid for `length` bytes while the
    // retained NSData object is alive in this scope.
    unsafe {
        let ptr: *const c_void = msg_send![&*body, bytes];
        let bytes = std::slice::from_raw_parts(ptr.cast::<u8>(), len);
        Ok(bytes.to_vec())
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn scheme_owners_do_not_replace_or_retain_other_frame_runtimes() {
        let (_first_bundle, first) = crate::frame::test_support::runtime();
        let (_second_bundle, second) = crate::frame::test_support::runtime();
        let first_weak = Arc::downgrade(&first);
        let second_weak = Arc::downgrade(&second);
        let owner = |runtime| SchemeHandlerIvars {
            runtime,
            executor: Arc::default(),
            tasks: super::super::tasks::MainTasks::new(),
            pending: Rc::default(),
            #[cfg(feature = "application-ipc")]
            state: None,
            #[cfg(feature = "application-ipc")]
            ipc: None,
        };
        let first = owner(first);
        let second = owner(second);
        assert!(!Arc::ptr_eq(&first.runtime, &second.runtime));
        drop(first);
        assert!(first_weak.upgrade().is_none());
        assert!(second_weak.upgrade().is_some());
        drop(second);
        assert!(second_weak.upgrade().is_none());
    }

    #[test]
    fn native_request_body_limit_is_independent_of_asset_length_limit() {
        use objc2_foundation::{NSData, NSMutableURLRequest};
        let url = NSURL::URLWithString(&NSString::from_str("webui://app/api")).unwrap();
        let request = NSMutableURLRequest::requestWithURL(&url);
        for (size, accepted) in [(1024 * 1024, true), (1024 * 1024 + 1, false)] {
            let body = NSData::with_bytes(&vec![1; size]);
            request.setHTTPBody(Some(&body));
            assert_eq!(request_body(&request).is_ok(), accepted);
        }
    }
}
