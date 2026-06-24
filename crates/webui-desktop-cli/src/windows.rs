// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use webui_desktop::{
    DesktopHttpMethod, DesktopProtocolRequest, DesktopProtocolResponse, DesktopRuntime,
    DEFAULT_MAX_ASSET_BYTES,
};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2, ICoreWebView2Controller,
    ICoreWebView2CustomSchemeRegistration, ICoreWebView2Environment,
    ICoreWebView2EnvironmentOptions, ICoreWebView2WebResourceRequest,
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
};
use webview2_com::{
    CoTaskMemPWSTR, CoreWebView2CustomSchemeRegistration, CoreWebView2EnvironmentOptions,
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    NavigationStartingEventHandler, WebResourceRequestedEventHandler,
};
use windows::core::{
    implement, w, Error as WindowsError, Result as WindowsResult, HRESULT, PCWSTR, PWSTR,
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

const APP_SCHEME: &str = "webui";
const APP_ORIGIN: &str = "webui://app";

/// Run a packaged WebUI desktop app on Windows using WebView2.
///
/// # Errors
///
/// Returns an error if packaged resources cannot be located or WebView2 cannot initialize.
pub fn run_packaged_app() -> Result<()> {
    let resources = packaged_resources_dir()?;
    let manifest =
        webui_desktop::DesktopBundleManifest::load(&resources.join("manifest.webui-desktop.json"))
            .with_context(|| "Failed to read packaged desktop manifest")?;
    let runtime = Arc::new(DesktopRuntime::from_bundle(resources)?);
    run_runtime(runtime, manifest.window)
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
    let _com = initialize_com()?;
    configure_dpi_awareness()?;

    let frame = FrameWindow::new(&window)?;
    let environment = create_environment().with_context(|| {
        "Failed to initialize WebView2; install the Microsoft Edge WebView2 Runtime or use a Windows image that includes it"
    })?;
    let controller = create_controller(&environment, frame.hwnd)?;
    let webview = unsafe { controller.CoreWebView2()? };
    configure_settings(&webview, window.devtools)?;
    register_navigation_guard(&webview)?;
    register_runtime_handler(&environment, &webview, runtime)?;
    set_controller_bounds(&controller, frame.hwnd)?;
    unsafe { controller.SetIsVisible(true)? };
    let state = Box::new(WindowState { controller });
    set_window_state(frame.hwnd, Some(state));

    unsafe {
        let _ = WindowsAndMessaging::ShowWindow(frame.hwnd, WindowsAndMessaging::SW_SHOW);
        let _ = Gdi::UpdateWindow(frame.hwnd);
        let _ = KeyboardAndMouse::SetFocus(Some(frame.hwnd));
    }
    let url = CoTaskMemPWSTR::from(startup_url().as_str());
    unsafe { webview.Navigate(*url.as_ref().as_pcwstr())? };
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

fn packaged_resources_dir() -> Result<std::path::PathBuf> {
    let exe = std::env::current_exe().context("Failed to locate webui-desktop executable")?;
    let root = exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Failed to locate desktop executable directory"))?;
    Ok(root.join("resources").join("webui"))
}

fn startup_url() -> String {
    let path = std::env::var("WEBUI_DESKTOP_START_PATH").unwrap_or_else(|_| "/".to_string());
    let path = if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    };
    let mut url = String::with_capacity(APP_ORIGIN.len() + path.len());
    url.push_str(APP_ORIGIN);
    url.push_str(&path);
    url
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

fn webview_environment_options() -> ICoreWebView2EnvironmentOptions {
    let scheme = CoreWebView2CustomSchemeRegistration::new(APP_SCHEME.to_string());
    // SAFETY: The registration object is local to environment creation. WebView2
    // copies the COM option values while creating the environment.
    unsafe {
        scheme.set_has_authority_component(true);
        scheme.set_treat_as_secure(true);
    }
    let scheme: ICoreWebView2CustomSchemeRegistration = scheme.into();
    let options = CoreWebView2EnvironmentOptions::default();
    // SAFETY: The vector contains a valid registration for the `webui://app`
    // authority used by startup and by every runtime-served resource.
    unsafe { options.set_scheme_registrations(vec![Some(scheme)]) };
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

fn register_navigation_guard(webview: &ICoreWebView2) -> Result<()> {
    unsafe {
        let mut token = 0_i64;
        webview.add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_sender, args| {
                if let Some(args) = args {
                    let uri = read_pwstr(|out| args.Uri(out))?;
                    if !is_allowed_navigation_url(&uri) {
                        args.SetCancel(true)?;
                    }
                }
                Ok(())
            })),
            &mut token,
        )?;
    }
    Ok(())
}

fn is_allowed_navigation_url(url: &str) -> bool {
    url == "about:blank" || url == APP_ORIGIN || url.starts_with("webui://app/")
}

fn register_runtime_handler(
    environment: &ICoreWebView2Environment,
    webview: &ICoreWebView2,
    runtime: Arc<DesktopRuntime>,
) -> Result<()> {
    unsafe {
        webview.AddWebResourceRequestedFilter(
            w!("webui://app/*"),
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
        )?;
        let environment = environment.clone();
        let mut token = 0_i64;
        webview.add_WebResourceRequested(
            &WebResourceRequestedEventHandler::create(Box::new(move |_sender, args| {
                if let Some(args) = args {
                    handle_web_resource_request(&environment, &runtime, &args)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
    }
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
