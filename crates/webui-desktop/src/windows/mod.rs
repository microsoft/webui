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
#[cfg(feature = "native-capture")]
pub(crate) mod capture;
#[cfg(feature = "native-clipboard")]
pub(crate) mod clipboard;
mod command;
mod create;
#[cfg(feature = "native-dialogs")]
pub(crate) mod dialogs;
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
#[cfg(feature = "local-server")]
mod local_controls;
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
#[cfg(feature = "local-server")]
use std::sync::atomic::{AtomicUsize, Ordering};
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
/// UI-only owner-loss wake, independent of the bounded window-command queue.
#[cfg(feature = "local-server")]
pub(super) const OWNER_LOST_MESSAGE: u32 = WindowsAndMessaging::WM_APP + 4;
#[cfg(feature = "local-server")]
static NEXT_OWNER_CLOSE_COOKIE: AtomicUsize = AtomicUsize::new(1);

#[cfg(feature = "local-server")]
fn next_owner_close_cookie() -> Result<usize> {
    NEXT_OWNER_CLOSE_COOKIE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| next.checked_add(1))
        .map_err(|_| anyhow::anyhow!("native window generation exhausted; restart the desktop host before opening another local-server frame"))
}

#[cfg(feature = "local-server")]
pub(super) fn post_owner_lost(
    hwnd: windows::Win32::Foundation::HWND,
    cookie: usize,
) -> std::result::Result<(), crate::HostCloseError> {
    // SAFETY: PostMessageW is thread-safe and never waits for the UI loop.
    // The receiver checks both its window generation and weak lifetime.
    unsafe {
        WindowsAndMessaging::PostMessageW(Some(hwnd), OWNER_LOST_MESSAGE, WPARAM(cookie), LPARAM(0))
    }
    .map_err(|error| crate::HostCloseError::WakeFailed {
        message: error.to_string(),
    })
}
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
    run_content(FrameContent::Bundle(frame))
}

#[cfg(feature = "local-server")]
pub(crate) fn run_local_server_frame(frame: crate::LocalServerFrame) -> Result<()> {
    run_content(FrameContent::Local(frame))
}

enum FrameContent {
    Bundle(DesktopFrame),
    #[cfg(feature = "local-server")]
    Local(crate::LocalServerFrame),
}

impl FrameContent {
    fn window(&self) -> &WindowOptions {
        match self {
            Self::Bundle(frame) => &frame.window,
            #[cfg(feature = "local-server")]
            Self::Local(frame) => &frame.window,
        }
    }
    fn app_id(&self) -> Option<&str> {
        match self {
            Self::Bundle(frame) => frame.app_id.as_deref(),
            #[cfg(feature = "local-server")]
            Self::Local(frame) => frame.app_id.as_deref(),
        }
    }
    fn events(&self) -> &crate::EventRegistry {
        match self {
            Self::Bundle(frame) => &frame.events,
            #[cfg(feature = "local-server")]
            Self::Local(frame) => &frame.events,
        }
    }
    fn window_handle(&self) -> &crate::WindowHandle {
        match self {
            Self::Bundle(frame) => &frame.window_handle,
            #[cfg(feature = "local-server")]
            Self::Local(frame) => &frame.window_handle,
        }
    }
    fn background(&self) -> Arc<crate::window::LiveBackground> {
        match self {
            Self::Bundle(frame) => frame.runtime.live_background(),
            #[cfg(feature = "local-server")]
            Self::Local(frame) => Arc::clone(&frame.live_background),
        }
    }
}

fn run_content(frame: FrameContent) -> Result<()> {
    #[cfg(feature = "local-server")]
    if let FrameContent::Local(local) = &frame {
        local.lifetime().require_active()?;
    }
    #[cfg(feature = "local-server")]
    let owner_close_cookie = match &frame {
        FrameContent::Bundle(_) => None,
        FrameContent::Local(_) => Some(next_owner_close_cookie()?),
    };
    let store = WindowStateStore::for_window(frame.window().remember_state, frame.app_id())?;
    let _com = initialize_com()?;
    configure_dpi_awareness()?;
    let runtime = app_sdk::Runtime::initialize()?;

    let saved = state::load_saved_state(store.as_ref());
    let window_frame = FrameWindow::new(frame.window(), saved.as_ref())?;
    let app_window = app_sdk::WindowFrame::attach(&runtime, window_frame.hwnd, frame.window())?;

    let profile = webview::browser_profile(frame.app_id())?;
    let environment = webview::create_environment(&profile.path).with_context(|| {
        "Failed to initialize WebView2; install the Microsoft Edge WebView2 Runtime or use a Windows image that includes it"
    })?;
    let content = window_frame.hwnd;
    let controller = webview::create_controller(&environment, content)?;
    webview::configure_controller_background(&controller, frame.window().background)?;
    webview::configure_window_effect(window_frame.hwnd, frame.window().effect);
    // SAFETY: The controller was created successfully, so it owns a WebView2.
    let webview = unsafe { controller.CoreWebView2()? };
    webview::configure_settings(&webview, frame.window().devtools)?;
    #[cfg(feature = "native-dialogs")]
    let dialog_services = match &frame {
        FrameContent::Bundle(_) => None,
        FrameContent::Local(local) => Some(local.native_services()?),
    };
    #[cfg(feature = "native-dialogs")]
    if let Some(services) = &dialog_services {
        services.attach_dialogs(window_frame.hwnd.0 as usize);
    }
    #[cfg(feature = "local-server")]
    let local_controls = match &frame {
        FrameContent::Local(local)
            if !matches!(frame.window().titlebar, crate::TitlebarStyle::Native) =>
        {
            Some(local_controls::LocalControls::new(
                local.origin().clone(),
                local.lifetime().clone(),
            ))
        }
        _ => None,
    };
    #[cfg(feature = "native-capture")]
    let capture_services = match &frame {
        FrameContent::Bundle(_) => None,
        FrameContent::Local(local) => Some(local.native_services()?),
    };
    #[cfg(feature = "native-capture")]
    let capture_registration = capture_services
        .as_ref()
        .map(|services| capture::install(&services.capture_for_revoke(), window_frame.hwnd))
        .transpose()?;
    #[cfg(feature = "native-clipboard")]
    if let Some(services) = &capture_services {
        services.attach_clipboard(window_frame.hwnd.0 as usize);
    }
    #[cfg(feature = "application-ipc")]
    let ipc = match &frame {
        FrameContent::Bundle(bundle) => Some(ipc::WindowsIpc::new(
            bundle.ipc_bridge(),
            &webview,
            window_frame.hwnd,
        )?),
        #[cfg(feature = "local-server")]
        FrameContent::Local(local) => local
            .ipc_bridge()
            .map(|bridge| {
                ipc::WindowsIpc::new_local(
                    bridge,
                    &webview,
                    window_frame.hwnd,
                    local.origin().clone(),
                    local.lifetime().clone(),
                )
            })
            .transpose()?,
    };
    #[cfg(feature = "application-ipc")]
    let _ipc_shutdown = ipc.as_ref().map(|ipc| ipc::Shutdown(Rc::clone(ipc)));

    let navigation_starting = webview::register_navigation_guard(
        &webview,
        frame.events().clone(),
        webview::NavigationGuardContext {
            #[cfg(feature = "native-capture")]
            capture: capture_services.clone(),
            #[cfg(feature = "local-server")]
            controls: local_controls.clone(),
            #[cfg(feature = "local-server")]
            origin: match &frame {
                FrameContent::Bundle(_) => None,
                FrameContent::Local(local) => Some(local.origin().clone()),
            },
            #[cfg(feature = "local-server")]
            lifetime: match &frame {
                FrameContent::Bundle(_) => None,
                FrameContent::Local(local) => Some(local.lifetime().clone()),
            },
        },
    )?;
    #[cfg(feature = "local-server")]
    let local_navigation = match (&frame, owner_close_cookie) {
        (FrameContent::Bundle(_), _) => None,
        (FrameContent::Local(local), Some(cookie)) => Some(webview::register_local_frame_guards(
            &webview,
            window_frame.hwnd,
            local.lifetime().clone(),
            cookie,
            Arc::clone(&local.frame_policy),
        )?),
        (FrameContent::Local(_), None) => {
            return Err(anyhow::anyhow!(
                "local-server window generation is missing; restart the desktop host"
            ));
        }
    };
    let navigation_completed = webview::register_navigation_completed(
        &webview,
        frame.events().clone(),
        window_frame.hwnd,
        frame.background(),
        webview::CompletionOwner {
            #[cfg(feature = "native-capture")]
            capture: capture_services.clone(),
            #[cfg(feature = "local-server")]
            controls: local_controls.clone(),
            #[cfg(feature = "local-server")]
            lifetime: owner_close_cookie.and_then(|cookie| match &frame {
                FrameContent::Bundle(_) => None,
                FrameContent::Local(local) => Some((local.lifetime().clone(), cookie)),
            }),
        },
    )?;
    if matches!(&frame, FrameContent::Bundle(_)) {
        webview::inject_drag_script(&webview)?;
    }
    // Local-server controls are a separate, document-nonce-bound host bridge,
    // never the packaged app's unconditional native command path.
    app_window.install_metrics(&webview)?;
    let controls = matches!(&frame, FrameContent::Bundle(_));
    let needs_message_handler = controls;
    #[cfg(feature = "local-server")]
    let needs_message_handler = needs_message_handler || local_controls.is_some();
    #[cfg(feature = "application-ipc")]
    let needs_message_handler = needs_message_handler || ipc.is_some();
    let web_message_received = if needs_message_handler {
        Some(bridge::register_message_handler(
            &webview,
            window_frame.hwnd,
            controls,
            #[cfg(feature = "local-server")]
            local_controls.clone(),
            #[cfg(feature = "application-ipc")]
            ipc.as_ref().map(Rc::downgrade).unwrap_or_default(),
        )?)
    } else {
        None
    };
    let application_tasks = tasks::ApplicationTasks::new(window_frame.hwnd);
    let web_resource_requested = match &frame {
        FrameContent::Bundle(bundle) => Some(protocol::register_runtime_handler(
            &environment,
            &webview,
            bundle,
            std::rc::Rc::downgrade(&application_tasks),
            #[cfg(feature = "application-ipc")]
            ipc.as_ref().map(Rc::downgrade).unwrap_or_default(),
        )?),
        #[cfg(feature = "local-server")]
        FrameContent::Local(_) => None,
    };
    #[cfg(feature = "local-server")]
    let owner_close_registration = match (&frame, owner_close_cookie) {
        (FrameContent::Bundle(_), _) => None,
        (FrameContent::Local(local), Some(cookie)) => {
            let handle = window_frame.hwnd.0 as usize;
            #[cfg(feature = "native-capture")]
            let capture = capture_services
                .as_ref()
                .map(|services| services.capture_for_revoke());
            #[cfg(feature = "native-clipboard")]
            let clipboard = capture_services
                .as_ref()
                .map(|services| services.clipboard_for_revoke());
            #[cfg(feature = "native-dialogs")]
            let dialogs = dialog_services
                .as_ref()
                .map(|services| services.dialogs_for_revoke());
            Some(local.lifetime().register_close_fallible(Arc::new(move || {
                #[cfg(feature = "native-capture")]
                if let Some(capture) = &capture {
                    // Retire bytes synchronously on owner revocation, before
                    // the asynchronous close wake and without waking Futures
                    // under HostLifetime's close lock.
                    capture.close();
                }
                #[cfg(feature = "native-clipboard")]
                if let Some(clipboard) = &clipboard {
                    // No future wake under HostLifetime's close lock.
                    clipboard.close_silent();
                }
                #[cfg(feature = "native-dialogs")]
                if let Some(dialogs) = &dialogs {
                    dialogs.close_silent();
                }
                let hwnd = windows::Win32::Foundation::HWND(handle as *mut std::ffi::c_void);
                post_owner_lost(hwnd, cookie)
            }))?)
        }
        (FrameContent::Local(_), None) => {
            return Err(anyhow::anyhow!(
                "local-server window generation is missing; restart the desktop host"
            ));
        }
    };
    message::set_controller_bounds(&controller, content)?;
    // SAFETY: The controller is live and owns the WebView2 surface.
    unsafe { controller.SetIsVisible(true)? };

    let state = Box::new(FrameState {
        app_window,
        content,
        application_tasks,
        #[cfg(feature = "application-ipc")]
        ipc: ipc.as_ref().map(Rc::clone),
        controller,
        #[cfg(feature = "native-dialogs")]
        dialogs: dialog_services
            .as_ref()
            .map(|services| services.dialogs_for_revoke()),
        #[cfg(feature = "native-clipboard")]
        clipboard: capture_services
            .as_ref()
            .map(|services| services.clipboard_for_revoke()),
        #[cfg(feature = "local-server")]
        local_controls,
        #[cfg(feature = "native-capture")]
        capture_registration,
        _navigation_starting: navigation_starting,
        #[cfg(feature = "local-server")]
        _local_navigation: local_navigation,
        #[cfg(feature = "local-server")]
        _owner_close_registration: owner_close_registration,
        #[cfg(feature = "local-server")]
        local_lifetime: match &frame {
            FrameContent::Bundle(_) => None,
            FrameContent::Local(local) => Some(local.lifetime().clone()),
        },
        #[cfg(feature = "local-server")]
        owner_close_cookie,
        _navigation_completed: navigation_completed,
        _web_message_received: web_message_received,
        _web_resource_requested: web_resource_requested,
        events: frame.events().clone(),
        window_handle: frame.window_handle().clone(),
        options: frame.window().clone(),
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
    if let Some(ipc) = &ipc {
        let script = if ipc.is_local() {
            #[cfg(feature = "local-server")]
            {
                crate::ipc_assets::LOCAL_NATIVE_BOOTSTRAP_SCRIPT
            }
            #[cfg(not(feature = "local-server"))]
            {
                crate::ipc_assets::NATIVE_BOOTSTRAP_SCRIPT
            }
        } else {
            crate::ipc_assets::NATIVE_BOOTSTRAP_SCRIPT
        };
        ipc.install(script)?;
    }
    #[cfg(feature = "local-server")]
    if let FrameContent::Local(local) = &frame {
        local.lifetime().require_active()?;
    }

    if frame.window().fullscreen {
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
    match &frame {
        FrameContent::Bundle(_) => webview::navigate_to_startup_url(&webview)?,
        #[cfg(feature = "local-server")]
        FrameContent::Local(local) => {
            local.lifetime().require_active()?;
            webview::navigate_to_url(&webview, &local.options.url())?;
        }
    }
    let message_loop_result = message::message_loop();
    let _ = frame.events().dispatch(&DesktopEvent::Exiting);
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
