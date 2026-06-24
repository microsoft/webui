// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::cell::OnceCell;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, OnceLock};

use crate::DesktopFrame;
use anyhow::{Context, Result};
use block2::DynBlock;
use objc2::ffi::NSInteger;
use objc2::ffi::NSUInteger;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::ProtocolObject;
use objc2::{
    define_class, extern_class, extern_conformance, extern_methods, msg_send, AnyThread,
    DefinedClass, MainThreadOnly,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSAutoresizingMaskOptions,
    NSBackingStoreType, NSWindow, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSData, NSDictionary, NSNotification, NSObject, NSObjectProtocol, NSPoint,
    NSRect, NSSize, NSString, NSURLRequest, NSURLResponse, NSURL,
};
use objc2_web_kit::{WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate};
use objc2_web_kit::{
    WKURLSchemeHandler, WKURLSchemeTask, WKWebView, WKWebViewConfiguration, WKWebsiteDataStore,
};
use webui_desktop::{
    DesktopHttpMethod, DesktopProtocolRequest, DesktopProtocolResponse, DesktopRuntime,
    DEFAULT_MAX_ASSET_BYTES,
};

static DESKTOP_RUNTIME: OnceLock<Mutex<Option<Arc<DesktopRuntime>>>> = OnceLock::new();

extern_class!(
    /// HTTP response subclass used to preserve status codes for custom-scheme loads.
    #[unsafe(super(NSURLResponse))]
    #[derive(Debug, PartialEq, Eq, Hash)]
    struct NSHTTPURLResponse;
);

extern_conformance!(
    // SAFETY: NSHTTPURLResponse inherits NSObjectProtocol conformance from Foundation.
    unsafe impl NSObjectProtocol for NSHTTPURLResponse {}
);

#[allow(non_snake_case)]
impl NSHTTPURLResponse {
    extern_methods!(
        #[unsafe(method(initWithURL:statusCode:HTTPVersion:headerFields:))]
        #[unsafe(method_family = init)]
        fn initWithURL_statusCode_HTTPVersion_headerFields(
            this: Allocated<Self>,
            url: &NSURL,
            status_code: NSInteger,
            http_version: Option<&NSString>,
            header_fields: Option<&NSDictionary<NSString, NSString>>,
        ) -> Retained<Self>;
    );
}

struct AppDelegateIvars {
    window: OnceCell<Retained<NSWindow>>,
    webview: OnceCell<Retained<WKWebView>>,
    scheme_handler: OnceCell<Retained<DesktopSchemeHandler>>,
    navigation_delegate: OnceCell<Retained<DesktopNavigationDelegate>>,
    title: Retained<NSString>,
    width: f64,
    height: f64,
    devtools: bool,
}

define_class!(
    // SAFETY: Delegate is an NSObject subclass with no Drop implementation.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = AppDelegateIvars]
    struct DesktopAppDelegate;

    // SAFETY: NSObjectProtocol has no additional safety requirements.
    unsafe impl NSObjectProtocol for DesktopAppDelegate {}

    // SAFETY: Method signatures match NSApplicationDelegate.
    unsafe impl NSApplicationDelegate for DesktopAppDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, notification: &NSNotification) {
            let mtm = self.mtm();
            let Some(app_obj) = notification.object() else {
                return;
            };
            let Ok(app) = app_obj.downcast::<NSApplication>() else {
                return;
            };

            let ivars = self.ivars();
            let rect = NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(ivars.width, ivars.height),
            );

            // SAFETY: NSWindow is allocated and initialized on the main thread,
            // with a valid content rect and standard style flags.
            let window = unsafe {
                NSWindow::initWithContentRect_styleMask_backing_defer(
                    NSWindow::alloc(mtm),
                    rect,
                    NSWindowStyleMask::Titled
                        | NSWindowStyleMask::Closable
                        | NSWindowStyleMask::Miniaturizable
                        | NSWindowStyleMask::Resizable,
                    NSBackingStoreType::Buffered,
                    false,
                )
            };
            // SAFETY: The window is retained in the delegate OnceCell, so it
            // must not auto-release itself when closed.
            unsafe { window.setReleasedWhenClosed(false) };
            window.setTitle(&ivars.title);

            let scheme_handler = DesktopSchemeHandler::new(mtm);
            let navigation_delegate = DesktopNavigationDelegate::new(mtm);
            // SAFETY: WKWebViewConfiguration::new and WKWebView initialization
            // must run on the main thread; mtm proves this. The frame is valid.
            let webview = unsafe {
                let config = WKWebViewConfiguration::new(mtm);
                // SAFETY: The handler object lives for the app lifetime via
                // `scheme_handler` OnceCell below, and WebKit calls it only on
                // the main thread for the registered custom scheme.
                config.setURLSchemeHandler_forURLScheme(
                    Some(ProtocolObject::from_ref(&*scheme_handler)),
                    &NSString::from_str("webui"),
                );
                config.setWebsiteDataStore(&WKWebsiteDataStore::nonPersistentDataStore(mtm));
                WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), rect, &config)
            };
            // SAFETY: `navigation_delegate` is retained in the delegate
            // OnceCell for the app lifetime. The policy implementation allows
            // only the custom app origin and cancels everything else.
            unsafe {
                webview
                    .setNavigationDelegate(Some(ProtocolObject::from_ref(&*navigation_delegate)));
            }
            if ivars.devtools || devtools_enabled_by_env() {
                // SAFETY: `setInspectable:` is a WebKit setter on a live
                // WKWebView created on the main thread. It only enables Safari
                // Web Inspector for this development webview.
                unsafe { webview.setInspectable(true) };
            }
            webview.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewHeightSizable,
            );
            window.setContentView(Some(&webview));
            window.center();
            window.setDelegate(Some(ProtocolObject::from_ref(self)));
            window.makeKeyAndOrderFront(None);

            if let Some(url) = NSURL::URLWithString(&NSString::from_str(&startup_url())) {
                let request = NSURLRequest::requestWithURL(&url);
                // SAFETY: The request URL uses the registered custom scheme.
                unsafe {
                    let _ = webview.loadRequest(&request);
                }
            }

            let _ = ivars.window.set(window);
            let _ = ivars.webview.set(webview);
            let _ = ivars.scheme_handler.set(scheme_handler);
            let _ = ivars.navigation_delegate.set(navigation_delegate);
            app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
            #[allow(deprecated)]
            app.activateIgnoringOtherApps(true);
        }
    }

    // SAFETY: Method signatures match NSWindowDelegate.
    unsafe impl NSWindowDelegate for DesktopAppDelegate {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &NSNotification) {
            // SAFETY: Called on the main thread by AppKit while the shared app exists.
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }
    }
);

#[derive(Debug, Default)]
struct NavigationDelegateIvars;

define_class!(
    // SAFETY: Navigation delegate is an NSObject subclass with no Drop implementation.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = NavigationDelegateIvars]
    struct DesktopNavigationDelegate;

    // SAFETY: NSObjectProtocol has no additional safety requirements.
    unsafe impl NSObjectProtocol for DesktopNavigationDelegate {}

    // SAFETY: Method signatures match WKNavigationDelegate.
    #[allow(non_snake_case)]
    unsafe impl WKNavigationDelegate for DesktopNavigationDelegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        unsafe fn webView_decidePolicyForNavigationAction_decisionHandler(
            &self,
            _web_view: &WKWebView,
            navigation_action: &WKNavigationAction,
            decision_handler: &DynBlock<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            let request = navigation_action.request();
            let policy = request
                .URL()
                .as_deref()
                .filter(|url| is_allowed_navigation_url(url))
                .map_or(WKNavigationActionPolicy::Cancel, |_| {
                    WKNavigationActionPolicy::Allow
                });
            decision_handler.call((policy,));
        }
    }
);

#[derive(Debug, Default)]
struct SchemeHandlerIvars;

define_class!(
    // SAFETY: Scheme handler is an NSObject subclass with no Drop implementation.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = SchemeHandlerIvars]
    struct DesktopSchemeHandler;

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
            handle_scheme_task(url_scheme_task);
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
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SchemeHandlerIvars);
        // SAFETY: NSObject init has the expected signature for this subclass.
        unsafe { msg_send![super(this), init] }
    }
}

impl DesktopNavigationDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(NavigationDelegateIvars);
        // SAFETY: NSObject init has the expected signature for this subclass.
        unsafe { msg_send![super(this), init] }
    }
}

impl DesktopAppDelegate {
    fn new(mtm: MainThreadMarker, options: MacosLaunchOptions) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(AppDelegateIvars {
            window: OnceCell::new(),
            webview: OnceCell::new(),
            scheme_handler: OnceCell::new(),
            navigation_delegate: OnceCell::new(),
            title: options.title,
            width: options.width,
            height: options.height,
            devtools: options.devtools,
        });
        // SAFETY: NSObject init has the expected signature for this subclass.
        unsafe { msg_send![super(this), init] }
    }
}

fn is_allowed_navigation_url(url: &NSURL) -> bool {
    let scheme = url.scheme().map(|value| value.to_string());
    match scheme.as_deref() {
        Some("webui") => url
            .host()
            .map(|value| value.to_string() == "app")
            .unwrap_or(false),
        Some("about") => true,
        _ => false,
    }
}

struct MacosLaunchOptions {
    title: Retained<NSString>,
    width: f64,
    height: f64,
    devtools: bool,
}

pub fn run_packaged_app() -> Result<()> {
    crate::run_packaged_app()
}

/// Run a prebuilt desktop runtime in a macOS WKWebView.
///
/// # Errors
///
/// Returns an error if AppKit cannot start on the main thread.
pub fn run_runtime(
    runtime: Arc<DesktopRuntime>,
    window: webui_desktop::WindowOptions,
) -> Result<()> {
    run_frame(DesktopFrame::new(runtime, window))
}

pub(crate) fn run_frame(frame: DesktopFrame) -> Result<()> {
    let mtm = MainThreadMarker::new().context("macOS desktop must run on the main thread")?;
    set_runtime(frame.runtime);
    run_app(mtm, frame.window)
}

fn run_app(mtm: MainThreadMarker, window: webui_desktop::WindowOptions) -> Result<()> {
    let app = NSApplication::sharedApplication(mtm);
    let title = NSString::from_str(&window.title);
    let delegate = DesktopAppDelegate::new(
        mtm,
        MacosLaunchOptions {
            title,
            width: f64::from(window.width),
            height: f64::from(window.height),
            devtools: window.devtools,
        },
    );
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.run();
    Ok(())
}

fn devtools_enabled_by_env() -> bool {
    std::env::var("WEBUI_DESKTOP_DEVTOOLS")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn startup_url() -> String {
    let path = std::env::var("WEBUI_DESKTOP_START_PATH").unwrap_or_else(|_| "/".to_string());
    let path = if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    };
    let mut url = String::with_capacity("webui://app".len() + path.len());
    url.push_str("webui://app");
    url.push_str(&path);
    if path.contains('?') {
        url.push('&');
    } else {
        url.push('?');
    }
    url.push_str("__webui_desktop_start=");
    url.push_str(&std::process::id().to_string());
    url
}

fn set_runtime(runtime: Arc<DesktopRuntime>) {
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

fn send_response(
    task: &ProtocolObject<dyn WKURLSchemeTask>,
    url: Option<&NSURL>,
    response: DesktopProtocolResponse,
) {
    let fallback_url = NSURL::URLWithString(&NSString::from_str("webui://app/"));
    let Some(url) = url.or(fallback_url.as_deref()) else {
        return;
    };
    let content_type_key = NSString::from_str("Content-Type");
    let content_type_value = NSString::from_str(&response.content_type);
    let cache_control_key = NSString::from_str("Cache-Control");
    let cache_control_value = NSString::from_str("no-store, no-cache, must-revalidate");
    let pragma_key = NSString::from_str("Pragma");
    let pragma_value = NSString::from_str("no-cache");
    let expires_key = NSString::from_str("Expires");
    let expires_value = NSString::from_str("0");
    let mut objects = [
        NonNull::from(&*content_type_value),
        NonNull::from(&*cache_control_value),
        NonNull::from(&*pragma_value),
        NonNull::from(&*expires_value),
    ];
    let mut keys = [
        NonNull::from(ProtocolObject::from_ref(&*content_type_key)),
        NonNull::from(ProtocolObject::from_ref(&*cache_control_key)),
        NonNull::from(ProtocolObject::from_ref(&*pragma_key)),
        NonNull::from(ProtocolObject::from_ref(&*expires_key)),
    ];
    // SAFETY: Keys and values are live NSString objects for the duration of
    // dictionary construction, and NSString conforms to NSCopying.
    let headers = unsafe {
        NSDictionary::<NSString, NSString>::dictionaryWithObjects_forKeys_count(
            objects.as_mut_ptr(),
            keys.as_mut_ptr(),
            NSUInteger::try_from(objects.len()).unwrap_or(0),
        )
    };
    let http_version = NSString::from_str("HTTP/1.1");
    let status_code = NSInteger::try_from(u32::from(response.status)).unwrap_or(500);
    let http_response = NSHTTPURLResponse::initWithURL_statusCode_HTTPVersion_headerFields(
        NSHTTPURLResponse::alloc(),
        url,
        status_code,
        Some(&http_version),
        Some(&headers),
    );
    // SAFETY: NSHTTPURLResponse is a Foundation subclass of NSURLResponse.
    let url_response: Retained<NSURLResponse> = unsafe { Retained::cast_unchecked(http_response) };
    // SAFETY: The task is live for the duration of the callback, and WebKit
    // requires didReceiveResponse, optional didReceiveData, then didFinish.
    unsafe {
        task.didReceiveResponse(&url_response);
        if !response.body.is_empty() {
            let data =
                NSData::dataWithBytes_length(response.body.as_ptr().cast(), response.body.len());
            task.didReceiveData(&data);
        }
        task.didFinish();
    }
}
