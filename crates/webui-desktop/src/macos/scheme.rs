// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Custom `webui://` URL-scheme handler that serves the runtime's protocol
//! responses directly to `WKWebView`, and the process-wide runtime handle it
//! dispatches requests through.

use std::ffi::c_void;
use std::sync::{Arc, Mutex, OnceLock};

use crate::{
    DesktopHttpMethod, DesktopProtocolRequest, DesktopProtocolResponse, DesktopRuntime,
    DEFAULT_MAX_ASSET_BYTES,
};
use objc2::rc::autoreleasepool;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString, NSURLRequest, NSURL};
use objc2_web_kit::{WKURLSchemeHandler, WKURLSchemeTask, WKWebView};

use super::response::send_response;

static DESKTOP_RUNTIME: OnceLock<Mutex<Option<Arc<DesktopRuntime>>>> = OnceLock::new();

/// Install the process-wide runtime used to answer scheme-task requests.
pub(super) fn set_runtime(runtime: Arc<DesktopRuntime>) {
    let slot = DESKTOP_RUNTIME.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = slot.lock() {
        *guard = Some(runtime);
    }
}

fn runtime() -> Option<Arc<DesktopRuntime>> {
    DESKTOP_RUNTIME
        .get()
        .and_then(|slot| slot.lock().ok())
        .and_then(|guard| guard.as_ref().cloned())
}

#[derive(Debug, Default)]
pub(super) struct SchemeHandlerIvars;

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
            autoreleasepool(|_| handle_scheme_task(url_scheme_task));
        }

        #[unsafe(method(webView:stopURLSchemeTask:))]
        unsafe fn webView_stopURLSchemeTask(
            &self,
            _web_view: &WKWebView,
            _url_scheme_task: &ProtocolObject<dyn WKURLSchemeTask>,
        ) {
        }
    }
);

impl DesktopSchemeHandler {
    pub(super) fn new(mtm: MainThreadMarker) -> objc2::rc::Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SchemeHandlerIvars);
        // SAFETY: NSObject init has the expected signature for this subclass.
        unsafe { msg_send![super(this), init] }
    }
}

fn handle_scheme_task(task: &ProtocolObject<dyn WKURLSchemeTask>) {
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

    let Some(runtime) = runtime() else {
        send_response(
            task,
            Some(&url),
            DesktopProtocolResponse::text(500, "Desktop runtime not initialized"),
        );
        return;
    };

    let desktop_request = DesktopProtocolRequest {
        method,
        path: &path,
        body: &body,
        wants_json,
    };
    let response = runtime
        .handle_request(&desktop_request)
        .unwrap_or_else(|err| DesktopProtocolResponse::text(500, err.chain_message()));
    send_response(task, Some(&url), response);
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
    if len > usize::try_from(DEFAULT_MAX_ASSET_BYTES).unwrap_or(usize::MAX) {
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
