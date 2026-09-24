// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Windows desktop backend built on Win32 and the WebView2 runtime.
//!
//! The backend keeps platform code in one place behind the cross-platform
//! [`crate::DesktopFrame`] contract:
//!
//! * [`create`] builds the native window, styles, and initial geometry.
//! * [`message`] owns the window procedure, custom frame, and lifecycle events.
//! * [`command`] executes queued window commands on the UI thread.
//! * [`state`] stores per-window state and persisted geometry.
//! * [`webview`] configures WebView2, navigation policy, and script bridges.
//! * [`bridge`] receives window controls and typed application IPC.
//! * [`protocol`] serves app resources through native request interception.

mod app_sdk;
mod bridge;
mod command;
mod create;
mod event;
#[cfg(feature = "application-ipc")]
mod ipc;
#[cfg(feature = "application-ipc")]
mod ipc_body;
#[cfg(feature = "application-ipc")]
mod ipc_control;
#[cfg(feature = "application-ipc")]
mod ipc_deadline;
#[cfg(feature = "application-ipc")]
mod ipc_http;
#[cfg(feature = "application-ipc")]
mod ipc_policy;
mod message;
mod nonclient;
mod protocol;
mod request;
mod state;
mod wakeup;
mod webview;

use std::cell::Cell;
#[cfg(feature = "application-ipc")]
use std::rc::Rc;
use std::sync::Arc;

use crate::{DesktopEvent, DesktopRuntime, WindowId, WindowOptions, WindowStateStore};
use anyhow::{Context, Result};
use windows::core::w;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{E_ACCESSDENIED, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::HiDpi;
use windows::Win32::UI::Input::KeyboardAndMouse;
use windows::Win32::UI::WindowsAndMessaging;

use crate::DesktopFrame;
use create::FrameWindow;
use state::FrameState;

/// Origin served to web content by the resource interceptor.
pub(super) const APP_ORIGIN: &str = "https://app.webui.localhost";
/// WebView2 normalizes HTTP URLs to include the authority's trailing slash.
pub(super) const APP_REQUEST_FILTER: PCWSTR = w!("https://app.webui.localhost/*");
/// Private message used to wake the UI thread for queued commands.
pub(super) const WAKE_MESSAGE: u32 = WindowsAndMessaging::WM_APP + 1;
pub(super) const APP_WAKE_MESSAGE: u32 = WindowsAndMessaging::WM_APP + 3;
mod tasks;
/// Coalesced, payload-free native IPC completion/control wake.
#[cfg(feature = "application-ipc")]
pub(super) const IPC_WAKE_MESSAGE: u32 = WindowsAndMessaging::WM_APP + 2;
/// Identity of the single window owned by this backend.
pub(super) const WINDOW_ID: WindowId = WindowId::PRIMARY;

/// Run a packaged WebUI desktop app on Windows using WebView2.
///
/// # Errors
///
/// Returns an error if packaged resources cannot be located or WebView2 cannot initialize.
pub fn run_packaged_app() -> Result<()> {
    crate::run_packaged_app().map_err(Into::into)
}

/// Run a prebuilt desktop runtime in a Windows WebView2 window.
///
/// # Errors
///
/// Returns an error if WebView2 cannot initialize.
pub fn run_runtime(runtime: Arc<DesktopRuntime>, window: WindowOptions) -> Result<()> {
    crate::run_runtime(runtime, window).map_err(Into::into)
}

/// Run a desktop frame until the native window closes.
///
/// # Errors
///
/// Returns an error if COM, the native window, or WebView2 cannot initialize.
pub(crate) fn run_frame(frame: DesktopFrame) -> Result<()> {
    let store = WindowStateStore::for_window(frame.window.remember_state, frame.app_id.as_deref())?;
    let _com = initialize_com()?;
    configure_dpi_awareness()?;
    let runtime = app_sdk::Runtime::initialize()?;

    let saved = state::load_saved_state(store.as_ref());
    let window_frame = FrameWindow::new(&frame.window, saved.as_ref())?;
    let app_window = app_sdk::WindowFrame::attach(&runtime, window_frame.hwnd, &frame.window)?;

    let profile = webview::browser_profile(frame.app_id.as_deref())?;
    let environment = webview::create_environment(&profile.path).with_context(|| {
        "Failed to initialize WebView2; install the Microsoft Edge WebView2 Runtime or use a Windows image that includes it"
    })?;
    let content = window_frame.hwnd;
    let controller = webview::create_controller(&environment, content)?;
    webview::configure_controller_background(&controller, frame.window.background)?;
    webview::configure_window_effect(window_frame.hwnd, frame.window.effect);
    // SAFETY: The controller was created successfully, so it owns a WebView2.
    let webview = unsafe { controller.CoreWebView2()? };
    webview::configure_settings(&webview, frame.window.devtools)?;
    #[cfg(feature = "application-ipc")]
    let ipc = ipc::WindowsIpc::new(frame.ipc_bridge(), &webview, window_frame.hwnd)?;
    #[cfg(feature = "application-ipc")]
    let _ipc_shutdown = ipc::Shutdown(Rc::clone(&ipc));

    let navigation_starting = webview::register_navigation_guard(&webview, frame.events.clone())?;
    let navigation_completed =
        webview::register_navigation_completed(&webview, frame.events.clone(), window_frame.hwnd)?;
    webview::inject_drag_script(&webview)?;
    app_window.install_metrics(&webview)?;
    let web_message_received = bridge::register_message_handler(
        &webview,
        window_frame.hwnd,
        #[cfg(feature = "application-ipc")]
        Rc::downgrade(&ipc),
    )?;
    let application_tasks = tasks::ApplicationTasks::new(window_frame.hwnd);
    let web_resource_requested = protocol::register_runtime_handler(
        &environment,
        &webview,
        &frame,
        std::rc::Rc::downgrade(&application_tasks),
        #[cfg(feature = "application-ipc")]
        Rc::downgrade(&ipc),
    )?;
    message::set_controller_bounds(&controller, content)?;
    // SAFETY: The controller is live and owns the WebView2 surface.
    unsafe { controller.SetIsVisible(true)? };

    let state = Box::new(FrameState {
        app_window,
        content,
        application_tasks,
        #[cfg(feature = "application-ipc")]
        ipc: Rc::clone(&ipc),
        controller,
        _navigation_starting: navigation_starting,
        _navigation_completed: navigation_completed,
        _web_message_received: web_message_received,
        _web_resource_requested: web_resource_requested,
        events: frame.events.clone(),
        window_handle: frame.window_handle.clone(),
        options: frame.window.clone(),
        webview: webview.clone(),
        store,
        fullscreen: Cell::new(false),
        window_state: Cell::new(state::initial_window_size_state(window_frame.hwnd)),
    });
    state::set_window_state(window_frame.hwnd, Some(state));
    // Installing a wakeup flushes an existing backlog. WebView2 initialization
    // pumps messages, so the native receiver must exist before attachment.
    install_wakeup(window_frame.hwnd)?;
    #[cfg(feature = "application-ipc")]
    ipc.install(crate::ipc_assets::NATIVE_BOOTSTRAP_SCRIPT)?;

    if frame.window.fullscreen {
        state::with_window_state(window_frame.hwnd, |state| {
            command::set_fullscreen(window_frame.hwnd, state, true);
        });
    }

    window_frame.show()?;
    message::refresh_frame(window_frame.hwnd);
    // SAFETY: `window_frame.hwnd` is a live window owned by this thread.
    unsafe {
        let _ = Gdi::UpdateWindow(window_frame.hwnd);
        let _ = KeyboardAndMouse::SetFocus(Some(window_frame.hwnd));
    }
    message::publish(window_frame.hwnd, &DesktopEvent::Ready);
    webview::navigate_to_startup_url(&webview)?;
    let message_loop_result = message::message_loop();
    let _ = frame.events.dispatch(&DesktopEvent::Exiting);
    message_loop_result
}

/// Wake the message pump whenever another thread queues a window command.
fn install_wakeup(hwnd: windows::Win32::Foundation::HWND) -> Result<()> {
    let receiver = state::with_window_state_result(hwnd, |state| state.window_handle.clone());
    // `HWND` is a raw pointer, so it is moved into the callback as an integer
    // and rebuilt on use, keeping the closure `Send + Sync` as the API requires.
    let handle = hwnd.0 as usize;
    wakeup::attach(receiver, move || {
        let hwnd = windows::Win32::Foundation::HWND(handle as *mut std::ffi::c_void);
        // SAFETY: `PostMessageW` is thread-safe by design. The window lives for
        // the whole message loop, and a stale handle after shutdown only makes
        // the post fail, which is ignored.
        let _ = unsafe {
            WindowsAndMessaging::PostMessageW(Some(hwnd), WAKE_MESSAGE, WPARAM(0), LPARAM(0))
        };
    })
}

/// COM apartment guard for the UI thread.
struct ComApartment;

impl Drop for ComApartment {
    fn drop(&mut self) {
        // SAFETY: `initialize_com` successfully initialized COM on this thread.
        unsafe { CoUninitialize() };
    }
}

/// Initialize a single-threaded COM apartment for WebView2.
fn initialize_com() -> Result<ComApartment> {
    // SAFETY: Called once at the start of the UI thread before WebView2 COM APIs.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
    Ok(ComApartment)
}

/// Opt the process into per-monitor DPI awareness.
fn configure_dpi_awareness() -> Result<()> {
    // SAFETY: Process-wide DPI awareness is configured before any window is created.
    match unsafe { HiDpi::SetProcessDpiAwareness(HiDpi::PROCESS_PER_MONITOR_DPI_AWARE) } {
        Ok(()) => Ok(()),
        Err(error) if error.code() == E_ACCESSDENIED => Ok(()),
        Err(error) => Err(error.into()),
    }
}
