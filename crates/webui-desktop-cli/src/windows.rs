// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use crate::DesktopFrame;
use anyhow::{Context, Result};
use serde_json::{Map, Value};
use webui_desktop::{
    DesktopHttpMethod, DesktopProtocolRequest, DesktopProtocolResponse, DesktopRuntime,
    DEFAULT_MAX_ASSET_BYTES,
};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2, ICoreWebView2Controller,
    ICoreWebView2Environment, ICoreWebView2EnvironmentOptions,
    ICoreWebView2NavigationStartingEventHandler, ICoreWebView2WebMessageReceivedEventHandler,
    ICoreWebView2WebResourceRequest, ICoreWebView2WebResourceRequestedEventHandler,
    ICoreWebView2_22, ICoreWebView2_3, COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_DENY,
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL, COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
};
use webview2_com::{
    AddScriptToExecuteOnDocumentCreatedCompletedHandler, CoTaskMemPWSTR,
    CoreWebView2EnvironmentOptions, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, NavigationStartingEventHandler,
    WebMessageReceivedEventHandler, WebResourceRequestedEventHandler,
};
use windows::core::{
    implement, w, Error as WindowsError, Interface, Result as WindowsResult, HRESULT, PCWSTR, PWSTR,
};
use windows::Win32::Foundation::{
    E_ACCESSDENIED, E_FAIL, E_INVALIDARG, E_NOTIMPL, E_POINTER, HINSTANCE, HWND, LPARAM, LRESULT,
    RECT, SIZE, S_OK, WPARAM,
};
use windows::Win32::Graphics::Gdi;
use windows::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, ISequentialStream_Impl, IStream, IStream as WinIStream,
    IStream_Impl, COINIT_APARTMENTTHREADED, LOCKTYPE, STATFLAG, STATSTG, STGC, STGTY_STREAM,
    STREAM_SEEK, STREAM_SEEK_CUR, STREAM_SEEK_END, STREAM_SEEK_SET,
};
use windows::Win32::System::LibraryLoader;
use windows::Win32::UI::HiDpi;
use windows::Win32::UI::Input::KeyboardAndMouse;
use windows::Win32::UI::WindowsAndMessaging::{
    self, MSG, WINDOW_LONG_PTR_INDEX, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};

const APP_ORIGIN: &str = "https://app.webui.localhost";
const APP_ORIGIN_PREFIX: &str = "https://app.webui.localhost/";
const APP_REQUEST_FILTER: PCWSTR = w!("*");
const APP_HOST: &str = "app.webui.localhost";
const FETCH_BRIDGE_KIND: &str = "webui-desktop-fetch";
const FETCH_BRIDGE_RESPONSE_KIND: &str = "webui-desktop-fetch-response";
const FETCH_BRIDGE_SCRIPT: &str = r#"
(() => {
  if (!window.chrome?.webview || window.__webuiDesktopFetchBridge) {
    return;
  }
  window.__webuiDesktopFetchBridge = true;
  const appOrigin = 'https://app.webui.localhost';
  const originalFetch = window.fetch.bind(window);
  let nextId = 1;
  const pending = new Map();

  const bytesToBase64 = (buffer) => {
    const bytes = new Uint8Array(buffer);
    let binary = '';
    for (let i = 0; i < bytes.length; i += 0x8000) {
      binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    }
    return btoa(binary);
  };

  const base64ToBytes = (value) => {
    const binary = atob(value || '');
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i += 1) {
      bytes[i] = binary.charCodeAt(i);
    }
    return bytes;
  };

  window.chrome.webview.addEventListener('message', (event) => {
    const message = event.data;
    if (!message || message.kind !== 'webui-desktop-fetch-response') {
      return;
    }
    const callbacks = pending.get(message.id);
    if (!callbacks) {
      return;
    }
    pending.delete(message.id);
    if (message.error) {
      callbacks.reject(new TypeError(message.error));
      return;
    }
    const emptyBodyStatus = message.status === 204 || message.status === 205 || message.status === 304;
    callbacks.resolve(new Response(emptyBodyStatus ? null : base64ToBytes(message.bodyBase64), {
      status: message.status,
      headers: message.headers || {}
    }));
  });

  window.fetch = async (input, init) => {
    const request = new Request(input, init);
    const url = new URL(request.url, location.href);
    if (url.origin !== appOrigin) {
      return originalFetch(input, init);
    }
    const id = nextId++;
    const headers = [];
    request.headers.forEach((value, key) => headers.push([key, value]));
    const hasBody = request.method !== 'GET' && request.method !== 'HEAD';
    const bodyBase64 = hasBody ? bytesToBase64(await request.clone().arrayBuffer()) : '';
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      window.chrome.webview.postMessage({
        kind: 'webui-desktop-fetch',
        id,
        method: request.method,
        url: url.href,
        headers,
        bodyBase64
      });
    });
  };
})();
"#;

/// Run a packaged WebUI desktop app on Windows using WebView2.
///
/// # Errors
///
/// Returns an error if packaged resources cannot be located or WebView2 cannot initialize.
pub fn run_packaged_app() -> Result<()> {
    crate::run_packaged_app()
}

/// Run a prebuilt desktop runtime in a Windows WebView2 window.
///
/// # Errors
///
/// Returns an error if WebView2 cannot initialize.
pub fn run_runtime(
    runtime: Arc<DesktopRuntime>,
    window: webui_desktop::WindowOptions,
) -> Result<()> {
    run_frame(DesktopFrame::new(runtime, window))
}

pub(crate) fn run_frame(frame: DesktopFrame) -> Result<()> {
    let _com = initialize_com()?;
    configure_dpi_awareness()?;

    let window_frame = FrameWindow::new(&frame.window)?;
    let environment = create_environment().with_context(|| {
        "Failed to initialize WebView2; install the Microsoft Edge WebView2 Runtime or use a Windows image that includes it"
    })?;
    let controller = create_controller(&environment, window_frame.hwnd)?;
    let webview = unsafe { controller.CoreWebView2()? };
    configure_settings(&webview, frame.window.devtools)?;
    let navigation_starting = register_navigation_guard(&webview)?;
    let web_message_received = register_fetch_bridge(&webview, Arc::clone(&frame.runtime))?;
    let web_resource_requested = register_runtime_handler(&environment, &webview, frame.runtime)?;
    let has_virtual_host_assets = register_virtual_host_assets(&webview)?;
    set_controller_bounds(&controller, window_frame.hwnd)?;
    unsafe { controller.SetIsVisible(true)? };
    let state = Box::new(WindowState {
        controller,
        _navigation_starting: navigation_starting,
        _web_message_received: web_message_received,
        _web_resource_requested: web_resource_requested,
    });
    set_window_state(window_frame.hwnd, Some(state));

    unsafe {
        let _ = WindowsAndMessaging::ShowWindow(window_frame.hwnd, WindowsAndMessaging::SW_SHOW);
        let _ = Gdi::UpdateWindow(window_frame.hwnd);
        let _ = KeyboardAndMouse::SetFocus(Some(window_frame.hwnd));
    }
    navigate_to_startup_url(&webview, has_virtual_host_assets)?;
    message_loop()
}

struct ComApartment;

impl Drop for ComApartment {
    fn drop(&mut self) {
        // SAFETY: `initialize_com` successfully initialized COM on this thread.
        unsafe { CoUninitialize() };
    }
}

fn initialize_com() -> Result<ComApartment> {
    // SAFETY: Called once at the start of the UI thread before WebView2 COM APIs.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
    Ok(ComApartment)
}

fn configure_dpi_awareness() -> Result<()> {
    // SAFETY: Process-wide DPI awareness is configured before any window is created.
    match unsafe { HiDpi::SetProcessDpiAwareness(HiDpi::PROCESS_PER_MONITOR_DPI_AWARE) } {
        Ok(()) => Ok(()),
        Err(error) if error.code() == E_ACCESSDENIED => Ok(()),
        Err(error) => Err(error.into()),
    }
}

struct FrameWindow {
    hwnd: HWND,
}

impl FrameWindow {
    fn new(window: &webui_desktop::WindowOptions) -> Result<Self> {
        let title = CoTaskMemPWSTR::from(window.title.as_str());
        let hwnd = unsafe {
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                lpszClassName: w!("WebUIDesktopWindow"),
                ..Default::default()
            };
            WindowsAndMessaging::RegisterClassW(&class);
            WindowsAndMessaging::CreateWindowExW(
                Default::default(),
                w!("WebUIDesktopWindow"),
                *title.as_ref().as_pcwstr(),
                WS_OVERLAPPEDWINDOW,
                WindowsAndMessaging::CW_USEDEFAULT,
                WindowsAndMessaging::CW_USEDEFAULT,
                i32::try_from(window.width).unwrap_or(1200),
                i32::try_from(window.height).unwrap_or(800),
                None,
                None,
                LibraryLoader::GetModuleHandleW(None)
                    .ok()
                    .map(|handle| HINSTANCE(handle.0)),
                None,
            )?
        };
        Ok(Self { hwnd })
    }
}

struct WindowState {
    controller: ICoreWebView2Controller,
    _navigation_starting: ICoreWebView2NavigationStartingEventHandler,
    _web_message_received: ICoreWebView2WebMessageReceivedEventHandler,
    _web_resource_requested: ICoreWebView2WebResourceRequestedEventHandler,
}

fn create_environment() -> Result<ICoreWebView2Environment> {
    let user_data_folder = webview_user_data_folder()?;
    let user_data_folder = user_data_folder.to_string_lossy();
    let user_data_folder = CoTaskMemPWSTR::from(user_data_folder.as_ref());
    let options = webview_environment_options();
    let (tx, rx) = mpsc::channel();
    CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| unsafe {
            CreateCoreWebView2EnvironmentWithOptions(
                PCWSTR::null(),
                *user_data_folder.as_ref().as_pcwstr(),
                &options,
                &handler,
            )
            .map_err(webview2_com::Error::WindowsError)
        }),
        Box::new(
            move |error_code, environment: Option<ICoreWebView2Environment>| {
                error_code?;
                tx.send(environment.ok_or_else(|| WindowsError::from(E_FAIL)))
                    .map_err(|_| WindowsError::from(E_FAIL))?;
                Ok(())
            },
        ),
    )?;
    rx.recv()
        .map_err(|_| anyhow::anyhow!("WebView2 environment creation was cancelled"))?
        .map_err(Into::into)
}

fn startup_url(use_packaged_index: bool) -> String {
    let default_path = if use_packaged_index {
        "/index.html"
    } else {
        "/"
    };
    let path =
        std::env::var("WEBUI_DESKTOP_START_PATH").unwrap_or_else(|_| default_path.to_string());
    startup_url_for_path(&path)
}

fn startup_url_for_path(path: &str) -> String {
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    let mut url = String::with_capacity(APP_ORIGIN.len() + path.len());
    url.push_str(APP_ORIGIN);
    url.push_str(&path);
    url
}

fn webview_environment_options() -> ICoreWebView2EnvironmentOptions {
    let options = CoreWebView2EnvironmentOptions::default();
    options.into()
}

fn webview_user_data_folder() -> Result<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let app_name = std::env::current_exe()
        .ok()
        .and_then(|path| path.file_stem().map(|stem| stem.to_os_string()))
        .and_then(|stem| stem.into_string().ok())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "webui-desktop".to_string());
    let dir = base
        .join("Microsoft")
        .join("WebUI")
        .join("Desktop")
        .join(app_name)
        .join("WebView2");
    std::fs::create_dir_all(&dir).with_context(|| {
        format!(
            "Failed to create WebView2 user data folder {}",
            dir.display()
        )
    })?;
    Ok(dir)
}

fn create_controller(
    environment: &ICoreWebView2Environment,
    hwnd: HWND,
) -> Result<ICoreWebView2Controller> {
    let (tx, rx) = mpsc::channel();
    let environment = environment.clone();
    CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| unsafe {
            environment
                .CreateCoreWebView2Controller(hwnd, &handler)
                .map_err(webview2_com::Error::WindowsError)
        }),
        Box::new(
            move |error_code, controller: Option<ICoreWebView2Controller>| {
                error_code?;
                tx.send(controller.ok_or_else(|| WindowsError::from(E_FAIL)))
                    .map_err(|_| WindowsError::from(E_FAIL))?;
                Ok(())
            },
        ),
    )?;
    rx.recv()
        .map_err(|_| anyhow::anyhow!("WebView2 controller creation was cancelled"))?
        .map_err(Into::into)
}

fn configure_settings(webview: &ICoreWebView2, devtools: bool) -> Result<()> {
    unsafe {
        let settings = webview.Settings()?;
        settings.SetAreDevToolsEnabled(devtools)?;
        settings.SetAreDefaultContextMenusEnabled(devtools)?;
    }
    Ok(())
}

fn register_navigation_guard(
    webview: &ICoreWebView2,
) -> Result<ICoreWebView2NavigationStartingEventHandler> {
    unsafe {
        let mut token = 0_i64;
        let handler = NavigationStartingEventHandler::create(Box::new(move |_sender, args| {
            if let Some(args) = args {
                let uri = read_pwstr(|out| args.Uri(out))?;
                if !is_allowed_navigation_url(&uri) {
                    args.SetCancel(true)?;
                }
            }
            Ok(())
        }));
        webview.add_NavigationStarting(&handler, &mut token)?;
        Ok(handler)
    }
}

fn is_allowed_navigation_url(url: &str) -> bool {
    url == "about:blank" || url == APP_ORIGIN || url.starts_with(APP_ORIGIN_PREFIX)
}

fn register_fetch_bridge(
    webview: &ICoreWebView2,
    runtime: Arc<DesktopRuntime>,
) -> Result<ICoreWebView2WebMessageReceivedEventHandler> {
    inject_fetch_bridge_script(webview)?;
    unsafe {
        let mut token = 0_i64;
        let handler = WebMessageReceivedEventHandler::create(Box::new(move |sender, args| {
            if let (Some(webview), Some(args)) = (sender, args) {
                handle_fetch_bridge_message(&webview, &runtime, &args)?;
            }
            Ok(())
        }));
        webview.add_WebMessageReceived(&handler, &mut token)?;
        Ok(handler)
    }
}

fn inject_fetch_bridge_script(webview: &ICoreWebView2) -> Result<()> {
    let webview = webview.clone();
    let script = CoTaskMemPWSTR::from(FETCH_BRIDGE_SCRIPT);
    AddScriptToExecuteOnDocumentCreatedCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| unsafe {
            webview
                .AddScriptToExecuteOnDocumentCreated(*script.as_ref().as_pcwstr(), &handler)
                .map_err(webview2_com::Error::WindowsError)
        }),
        Box::new(|result, _script_id| {
            result?;
            Ok(())
        }),
    )?;
    Ok(())
}

fn handle_fetch_bridge_message(
    webview: &ICoreWebView2,
    runtime: &DesktopRuntime,
    args: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2WebMessageReceivedEventArgs,
) -> WindowsResult<()> {
    let raw = read_pwstr(|out| unsafe { args.WebMessageAsJson(out) })?;
    let Ok(message) = serde_json::from_str::<Value>(&raw) else {
        return Ok(());
    };
    if message.get("kind").and_then(Value::as_str) != Some(FETCH_BRIDGE_KIND) {
        return Ok(());
    }

    let id = message
        .get("id")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let response = match fetch_bridge_response(runtime, &message) {
        Ok(response) => response,
        Err(error) => fetch_bridge_error_response(id, error),
    };
    post_web_message_json(webview, &response)
}

fn fetch_bridge_response(
    runtime: &DesktopRuntime,
    message: &Value,
) -> std::result::Result<Value, String> {
    let id = message
        .get("id")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET");
    let url = message
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| "desktop fetch bridge request is missing a URL".to_string())?;
    let body = message
        .get("bodyBase64")
        .and_then(Value::as_str)
        .map(decode_base64)
        .transpose()?
        .unwrap_or_default();
    let accept = header_value(message.get("headers"), "accept");
    let path = webui_path_from_uri(url);
    let request = DesktopProtocolRequest {
        method: DesktopHttpMethod::parse(method),
        path: &path,
        body: &body,
        wants_json: accept.contains("json") || accept.contains("ndjson"),
    };
    let response = runtime
        .handle_request(&request)
        .map_err(|error| error.chain_message())?;
    Ok(fetch_bridge_success_response(id, response))
}

fn fetch_bridge_error_response(id: u64, error: String) -> Value {
    let mut map = Map::new();
    map.insert(
        "kind".to_string(),
        Value::String(FETCH_BRIDGE_RESPONSE_KIND.to_string()),
    );
    map.insert("id".to_string(), Value::from(id));
    map.insert("error".to_string(), Value::String(error));
    Value::Object(map)
}

fn fetch_bridge_success_response(id: u64, response: DesktopProtocolResponse) -> Value {
    let mut headers = Map::new();
    headers.insert(
        "content-type".to_string(),
        Value::String(response.content_type),
    );

    let mut map = Map::new();
    map.insert(
        "kind".to_string(),
        Value::String(FETCH_BRIDGE_RESPONSE_KIND.to_string()),
    );
    map.insert("id".to_string(), Value::from(id));
    map.insert(
        "status".to_string(),
        Value::from(u64::from(response.status)),
    );
    map.insert("headers".to_string(), Value::Object(headers));
    map.insert(
        "bodyBase64".to_string(),
        Value::String(encode_base64(&response.body)),
    );
    Value::Object(map)
}

fn header_value(headers: Option<&Value>, name: &str) -> String {
    let Some(headers) = headers.and_then(Value::as_array) else {
        return String::new();
    };
    for pair in headers {
        let Some(pair) = pair.as_array() else {
            continue;
        };
        if pair.len() != 2 {
            continue;
        }
        let key = pair[0].as_str().unwrap_or_default();
        if key.eq_ignore_ascii_case(name) {
            return pair[1].as_str().unwrap_or_default().to_string();
        }
    }
    String::new()
}

fn post_web_message_json(webview: &ICoreWebView2, value: &Value) -> WindowsResult<()> {
    let text = serde_json::to_string(value).map_err(|_| WindowsError::from(E_FAIL))?;
    let text = CoTaskMemPWSTR::from(text.as_str());
    unsafe { webview.PostWebMessageAsJson(*text.as_ref().as_pcwstr()) }
}

fn register_runtime_handler(
    environment: &ICoreWebView2Environment,
    webview: &ICoreWebView2,
    runtime: Arc<DesktopRuntime>,
) -> Result<ICoreWebView2WebResourceRequestedEventHandler> {
    unsafe {
        register_web_resource_filter(webview)?;
        let environment = environment.clone();
        let mut token = 0_i64;
        let handler = WebResourceRequestedEventHandler::create(Box::new(move |_sender, args| {
            if let Some(args) = args {
                handle_web_resource_request(&environment, &runtime, &args)?;
            }
            Ok(())
        }));
        webview.add_WebResourceRequested(&handler, &mut token)?;
        Ok(handler)
    }
}

unsafe fn register_web_resource_filter(webview: &ICoreWebView2) -> Result<()> {
    if let Ok(webview) = webview.cast::<ICoreWebView2_22>() {
        webview.AddWebResourceRequestedFilterWithRequestSourceKinds(
            APP_REQUEST_FILTER,
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
            COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
        )?;
        return Ok(());
    }

    webview
        .AddWebResourceRequestedFilter(APP_REQUEST_FILTER, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?;
    Ok(())
}

fn register_virtual_host_assets(webview: &ICoreWebView2) -> Result<bool> {
    let Some(resources) = crate::find_packaged_resources_dir() else {
        return Ok(false);
    };
    let assets = resources.join("assets");
    if !assets.is_dir() {
        return Ok(false);
    }

    let webview: ICoreWebView2_3 = webview.cast()?;
    let host = CoTaskMemPWSTR::from(APP_HOST);
    let assets = assets.to_string_lossy();
    let assets = CoTaskMemPWSTR::from(assets.as_ref());
    unsafe {
        webview.SetVirtualHostNameToFolderMapping(
            *host.as_ref().as_pcwstr(),
            *assets.as_ref().as_pcwstr(),
            COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_DENY,
        )?;
    }
    Ok(true)
}

fn navigate_to_startup_url(webview: &ICoreWebView2, use_packaged_index: bool) -> Result<()> {
    let url = startup_url(use_packaged_index);
    let url = CoTaskMemPWSTR::from(url.as_str());
    unsafe { webview.Navigate(*url.as_ref().as_pcwstr())? };
    Ok(())
}

fn handle_web_resource_request(
    environment: &ICoreWebView2Environment,
    runtime: &DesktopRuntime,
    args: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2WebResourceRequestedEventArgs,
) -> WindowsResult<()> {
    unsafe {
        let request = args.Request()?;
        let uri = read_pwstr(|out| request.Uri(out))?;
        let method = read_pwstr(|out| request.Method(out))?;
        let method = DesktopHttpMethod::parse(&method);
        let headers = request.Headers()?;
        let accept = read_header(&headers, "Accept").unwrap_or_default();
        let body = read_request_body(&request)?;
        let path = webui_path_from_uri(&uri);
        let desktop_request = DesktopProtocolRequest {
            method,
            path: &path,
            body: &body,
            wants_json: accept.contains("json") || accept.contains("ndjson"),
        };
        let response = runtime
            .handle_request(&desktop_request)
            .unwrap_or_else(|err| DesktopProtocolResponse::text(500, err.chain_message()));
        let response = create_webview_response(environment, response)?;
        args.SetResponse(&response)?;
    }
    Ok(())
}

fn read_header(
    headers: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2HttpRequestHeaders,
    name: &str,
) -> Option<String> {
    let name = CoTaskMemPWSTR::from(name);
    read_pwstr(|out| unsafe { headers.GetHeader(*name.as_ref().as_pcwstr(), out) }).ok()
}

fn read_pwstr<F>(read: F) -> WindowsResult<String>
where
    F: FnOnce(*mut PWSTR) -> WindowsResult<()>,
{
    let mut raw = PWSTR::null();
    read(&mut raw)?;
    let value = CoTaskMemPWSTR::from(raw).to_string();
    Ok(value)
}

fn webui_path_from_uri(uri: &str) -> String {
    let rest = uri.strip_prefix(APP_ORIGIN).unwrap_or(uri);
    if rest.is_empty() {
        "/".to_string()
    } else {
        rest.to_string()
    }
}

fn encode_base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut chunks = bytes.chunks_exact(3);
    for chunk in &mut chunks {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
        out.push(base64_char((n >> 18) & 0x3f));
        out.push(base64_char((n >> 12) & 0x3f));
        out.push(base64_char((n >> 6) & 0x3f));
        out.push(base64_char(n & 0x3f));
    }
    let rem = chunks.remainder();
    if rem.len() == 1 {
        let n = u32::from(rem[0]) << 16;
        out.push(base64_char((n >> 18) & 0x3f));
        out.push(base64_char((n >> 12) & 0x3f));
        out.push('=');
        out.push('=');
    } else if rem.len() == 2 {
        let n = (u32::from(rem[0]) << 16) | (u32::from(rem[1]) << 8);
        out.push(base64_char((n >> 18) & 0x3f));
        out.push(base64_char((n >> 12) & 0x3f));
        out.push(base64_char((n >> 6) & 0x3f));
        out.push('=');
    }
    out
}

fn base64_char(index: u32) -> char {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    usize::try_from(index)
        .ok()
        .and_then(|index| TABLE.get(index))
        .map_or('A', |byte| char::from(*byte))
}

fn decode_base64(input: &str) -> std::result::Result<Vec<u8>, String> {
    let bytes = input.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return Err("desktop fetch bridge body is not valid base64".to_string());
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks_exact(4) {
        decode_base64_group(chunk, &mut out)?;
    }
    Ok(out)
}

fn decode_base64_group(group: &[u8], out: &mut Vec<u8>) -> std::result::Result<(), String> {
    let a = base64_value(group[0])?;
    let b = base64_value(group[1])?;
    let c = base64_value_or_padding(group[2])?;
    let d = base64_value_or_padding(group[3])?;
    if a == 64 || b == 64 || (c == 64 && d != 64) {
        return Err("desktop fetch bridge body has invalid base64 padding".to_string());
    }

    let c_bits = if c == 64 { 0 } else { c };
    let d_bits = if d == 64 { 0 } else { d };
    let n =
        (u32::from(a) << 18) | (u32::from(b) << 12) | (u32::from(c_bits) << 6) | u32::from(d_bits);
    out.push(base64_decoded_byte((n >> 16) & 0xff)?);
    if c != 64 {
        out.push(base64_decoded_byte((n >> 8) & 0xff)?);
    }
    if d != 64 {
        out.push(base64_decoded_byte(n & 0xff)?);
    }
    Ok(())
}

fn base64_decoded_byte(value: u32) -> std::result::Result<u8, String> {
    u8::try_from(value).map_err(|_| "desktop fetch bridge body is too large".to_string())
}

fn base64_value_or_padding(byte: u8) -> std::result::Result<u8, String> {
    if byte == b'=' {
        Ok(64)
    } else {
        base64_value(byte)
    }
}

fn base64_value(byte: u8) -> std::result::Result<u8, String> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err("desktop fetch bridge body contains invalid base64".to_string()),
    }
}

fn read_request_body(request: &ICoreWebView2WebResourceRequest) -> WindowsResult<Vec<u8>> {
    // SAFETY: The COM request object is valid for the duration of the
    // WebResourceRequested callback. A null content stream means an empty body.
    match unsafe { request.Content() } {
        Ok(stream) => read_request_stream(stream),
        Err(error) if error.code() == E_POINTER => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn read_request_stream(stream: WinIStream) -> WindowsResult<Vec<u8>> {
    let mut out = Vec::new();
    let max = usize::try_from(DEFAULT_MAX_ASSET_BYTES).unwrap_or(usize::MAX);
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let mut read = 0_u32;
        // SAFETY: `buffer` is valid for `buffer.len()` bytes and `read` points
        // to writable stack storage for the byte count.
        let hr = unsafe {
            stream.Read(
                buffer.as_mut_ptr().cast::<c_void>(),
                u32::try_from(buffer.len()).unwrap_or(u32::MAX),
                Some(&mut read),
            )
        };
        hr.ok()?;
        if read == 0 {
            break;
        }
        let read = usize::try_from(read).map_err(|_| WindowsError::from(E_INVALIDARG))?;
        if out.len().saturating_add(read) > max {
            return Err(WindowsError::from(E_INVALIDARG));
        }
        out.extend_from_slice(&buffer[..read]);
    }
    Ok(out)
}

fn create_webview_response(
    environment: &ICoreWebView2Environment,
    response: DesktopProtocolResponse,
) -> WindowsResult<webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2WebResourceResponse>
{
    let headers = response_headers(&response.content_type);
    let reason = status_reason(response.status);
    let status = i32::from(response.status);
    let stream: WinIStream = MemoryStream::new(response.body).into();
    let reason = CoTaskMemPWSTR::from(reason);
    let headers = CoTaskMemPWSTR::from(headers.as_str());
    unsafe {
        environment.CreateWebResourceResponse(
            &stream,
            status,
            *reason.as_ref().as_pcwstr(),
            *headers.as_ref().as_pcwstr(),
        )
    }
}

fn response_headers(content_type: &str) -> String {
    let mut headers = String::with_capacity(content_type.len() + 96);
    headers.push_str("Content-Type: ");
    headers.push_str(content_type);
    headers
        .push_str("\r\nCache-Control: no-store, no-cache, must-revalidate\r\nPragma: no-cache\r\n");
    headers
}

fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        _ => "OK",
    }
}

fn set_controller_bounds(controller: &ICoreWebView2Controller, hwnd: HWND) -> Result<()> {
    let size = get_window_size(hwnd);
    unsafe {
        controller.SetBounds(RECT {
            left: 0,
            top: 0,
            right: size.cx,
            bottom: size.cy,
        })?;
    }
    Ok(())
}

fn message_loop() -> Result<()> {
    let mut msg = MSG::default();
    loop {
        let result = unsafe { WindowsAndMessaging::GetMessageW(&mut msg, None, 0, 0).0 };
        match result {
            -1 => return Err(WindowsError::from_thread().into()),
            0 => return Ok(()),
            _ => unsafe {
                let _ = WindowsAndMessaging::TranslateMessage(&msg);
                WindowsAndMessaging::DispatchMessageW(&msg);
            },
        }
    }
}

extern "system" fn window_proc(hwnd: HWND, msg: u32, w_param: WPARAM, l_param: LPARAM) -> LRESULT {
    match msg {
        WindowsAndMessaging::WM_SIZE => {
            with_window_state(hwnd, |state| {
                let _ = set_controller_bounds(&state.controller, hwnd);
            });
            LRESULT::default()
        }
        WindowsAndMessaging::WM_CLOSE => {
            unsafe {
                let _ = WindowsAndMessaging::DestroyWindow(hwnd);
            }
            LRESULT::default()
        }
        WindowsAndMessaging::WM_DESTROY => {
            set_window_state(hwnd, None);
            unsafe { WindowsAndMessaging::PostQuitMessage(0) };
            LRESULT::default()
        }
        _ => unsafe { WindowsAndMessaging::DefWindowProcW(hwnd, msg, w_param, l_param) },
    }
}

fn get_window_size(hwnd: HWND) -> SIZE {
    let mut client_rect = RECT::default();
    let _ = unsafe { WindowsAndMessaging::GetClientRect(hwnd, &mut client_rect) };
    SIZE {
        cx: client_rect.right - client_rect.left,
        cy: client_rect.bottom - client_rect.top,
    }
}

fn set_window_state(hwnd: HWND, state: Option<Box<WindowState>>) -> Option<Box<WindowState>> {
    let value = state.map_or(0_isize, |state| Box::into_raw(state) as isize);
    let previous = unsafe { set_window_long(hwnd, WindowsAndMessaging::GWLP_USERDATA, value) };
    if previous == 0 {
        None
    } else {
        Some(unsafe { Box::from_raw(previous as *mut WindowState) })
    }
}

fn with_window_state(hwnd: HWND, f: impl FnOnce(&WindowState)) {
    let data = unsafe { get_window_long(hwnd, WindowsAndMessaging::GWLP_USERDATA) };
    if data != 0 {
        // SAFETY: `data` was stored by `set_window_state` from a live
        // `Box<WindowState>` and remains valid until WM_DESTROY clears it.
        let state = unsafe { &*(data as *const WindowState) };
        f(state);
    }
}

#[allow(non_snake_case)]
#[cfg(target_pointer_width = "32")]
unsafe fn set_window_long(hwnd: HWND, index: WINDOW_LONG_PTR_INDEX, value: isize) -> isize {
    unsafe { WindowsAndMessaging::SetWindowLongW(hwnd, index, value as _) as _ }
}

#[allow(non_snake_case)]
#[cfg(target_pointer_width = "64")]
unsafe fn set_window_long(hwnd: HWND, index: WINDOW_LONG_PTR_INDEX, value: isize) -> isize {
    unsafe { WindowsAndMessaging::SetWindowLongPtrW(hwnd, index, value) }
}

#[allow(non_snake_case)]
#[cfg(target_pointer_width = "32")]
unsafe fn get_window_long(hwnd: HWND, index: WINDOW_LONG_PTR_INDEX) -> isize {
    unsafe { WindowsAndMessaging::GetWindowLongW(hwnd, index) as _ }
}

#[allow(non_snake_case)]
#[cfg(target_pointer_width = "64")]
unsafe fn get_window_long(hwnd: HWND, index: WINDOW_LONG_PTR_INDEX) -> isize {
    unsafe { WindowsAndMessaging::GetWindowLongPtrW(hwnd, index) }
}

#[implement(IStream)]
struct MemoryStream {
    data: Arc<[u8]>,
    position: Mutex<usize>,
}

impl MemoryStream {
    fn new(data: Vec<u8>) -> Self {
        Self {
            data: data.into(),
            position: Mutex::new(0),
        }
    }
}

impl ISequentialStream_Impl for MemoryStream_Impl {
    fn Read(&self, pv: *mut c_void, cb: u32, pcbread: *mut u32) -> HRESULT {
        let Ok(mut position) = self.position.lock() else {
            return E_FAIL;
        };
        let remaining = self.data.len().saturating_sub(*position);
        let requested = usize::try_from(cb).unwrap_or(usize::MAX);
        let count = remaining.min(requested);
        if count != 0 && pv.is_null() {
            return E_POINTER;
        }
        if count != 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    self.data[*position..].as_ptr(),
                    pv.cast::<u8>(),
                    count,
                );
            }
            *position += count;
        }
        if !pcbread.is_null() {
            unsafe {
                *pcbread = u32::try_from(count).unwrap_or(u32::MAX);
            }
        }
        S_OK
    }

    fn Write(&self, _pv: *const c_void, _cb: u32, _pcbwritten: *mut u32) -> HRESULT {
        E_NOTIMPL
    }
}

impl IStream_Impl for MemoryStream_Impl {
    fn Seek(
        &self,
        dlibmove: i64,
        dworigin: STREAM_SEEK,
        plibnewposition: *mut u64,
    ) -> WindowsResult<()> {
        let mut position = self
            .position
            .lock()
            .map_err(|_| WindowsError::from(E_FAIL))?;
        let base = match dworigin {
            STREAM_SEEK_SET => 0_i64,
            STREAM_SEEK_CUR => {
                i64::try_from(*position).map_err(|_| WindowsError::from(E_INVALIDARG))?
            }
            STREAM_SEEK_END => {
                i64::try_from(self.data.len()).map_err(|_| WindowsError::from(E_INVALIDARG))?
            }
            _ => return Err(WindowsError::from(E_INVALIDARG)),
        };
        let next = base
            .checked_add(dlibmove)
            .ok_or_else(|| WindowsError::from(E_INVALIDARG))?;
        if next < 0 {
            return Err(WindowsError::from(E_INVALIDARG));
        }
        *position = usize::try_from(next).map_err(|_| WindowsError::from(E_INVALIDARG))?;
        if !plibnewposition.is_null() {
            unsafe {
                *plibnewposition =
                    u64::try_from(*position).map_err(|_| WindowsError::from(E_INVALIDARG))?;
            }
        }

        Ok(())
    }

    fn SetSize(&self, _libnewsize: u64) -> WindowsResult<()> {
        Err(WindowsError::from(E_NOTIMPL))
    }

    fn CopyTo(
        &self,
        _pstm: windows::core::Ref<IStream>,
        _cb: u64,
        _pcbread: *mut u64,
        _pcbwritten: *mut u64,
    ) -> WindowsResult<()> {
        Err(WindowsError::from(E_NOTIMPL))
    }

    fn Commit(&self, _grfcommitflags: &STGC) -> WindowsResult<()> {
        Ok(())
    }

    fn Revert(&self) -> WindowsResult<()> {
        Err(WindowsError::from(E_NOTIMPL))
    }

    fn LockRegion(&self, _liboffset: u64, _cb: u64, _dwlocktype: &LOCKTYPE) -> WindowsResult<()> {
        Err(WindowsError::from(E_NOTIMPL))
    }

    fn UnlockRegion(&self, _liboffset: u64, _cb: u64, _dwlocktype: u32) -> WindowsResult<()> {
        Err(WindowsError::from(E_NOTIMPL))
    }

    fn Stat(&self, pstatstg: *mut STATSTG, _grfstatflag: &STATFLAG) -> WindowsResult<()> {
        if pstatstg.is_null() {
            return Err(WindowsError::from(E_INVALIDARG));
        }
        unsafe {
            (*pstatstg) = STATSTG::default();
            (*pstatstg).r#type = STGTY_STREAM.0 as u32;
            (*pstatstg).cbSize =
                u64::try_from(self.data.len()).map_err(|_| WindowsError::from(E_INVALIDARG))?;
        }
        Ok(())
    }

    fn Clone(&self) -> WindowsResult<IStream> {
        Ok(MemoryStream {
            data: Arc::clone(&self.data),
            position: Mutex::new(0),
        }
        .into())
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn startup_url_uses_intercepted_https_origin() {
        assert_eq!(startup_url(false), "https://app.webui.localhost/");
        assert_eq!(startup_url(true), "https://app.webui.localhost/index.html");
        assert_eq!(
            startup_url_for_path("contacts"),
            "https://app.webui.localhost/contacts"
        );
    }

    #[test]
    fn navigation_guard_allows_only_app_origin() {
        assert!(is_allowed_navigation_url("about:blank"));
        assert!(is_allowed_navigation_url("https://app.webui.localhost"));
        assert!(is_allowed_navigation_url(
            "https://app.webui.localhost/contacts"
        ));
        assert!(!is_allowed_navigation_url(
            "data:text/html;charset=utf-8,%3Chtml%3E"
        ));
        assert!(!is_allowed_navigation_url("webui://app/"));
        assert!(!is_allowed_navigation_url("https://example.com/"));
    }

    #[test]
    fn request_uri_maps_to_runtime_path() {
        assert_eq!(webui_path_from_uri("https://app.webui.localhost"), "/");
        assert_eq!(
            webui_path_from_uri("https://app.webui.localhost/contacts?view=all"),
            "/contacts?view=all"
        );
    }

    #[test]
    fn fetch_bridge_base64_round_trips_body_bytes() {
        let body = b"{\"name\":\"Sarah\"}\0\xff";
        let encoded = encode_base64(body);
        assert_eq!(decode_base64(&encoded).unwrap(), body);
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
    }
}
