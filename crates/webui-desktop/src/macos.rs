// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! macOS desktop shell backend.
//!
//! This module is a slim coordinator: it owns process bootstrap
//! (`run_frame`/`run_app`) and the small set of free functions shared across
//! submodules. Each native peripheral lives in its own file under
//! `src/macos/` to keep cognitive complexity low and each concern
//! independently testable. See the module list below for the file layout.

use std::sync::Arc;

use crate::DesktopFrame;
use crate::{
    DesktopEvent, DesktopRuntime, DesktopShellConfig, EventRegistry, EventResponse, WindowOptions,
    WindowStateStore,
};
use anyhow::{Context, Result};
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::ProtocolObject;
use objc2::DefinedClass;
use objc2_app_kit::NSApplication;
#[cfg(feature = "local-server")]
use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
use objc2_foundation::{MainThreadMarker, NSString};
use objc2_web_kit::WKWebView;

mod app_delegate;
#[cfg(feature = "native-capture")]
pub(crate) mod capture;
#[cfg(feature = "native-clipboard")]
pub(crate) mod clipboard;
mod commands;
mod effects;
mod geometry;
mod host_message;
mod icon;
#[cfg(feature = "application-ipc")]
mod ipc;
#[cfg(feature = "application-ipc")]
mod ipc_control;
#[cfg(all(feature = "application-ipc", feature = "local-server"))]
mod ipc_data_message;
#[cfg(feature = "application-ipc")]
mod ipc_message;
#[cfg(feature = "application-ipc")]
mod ipc_scheme;
#[cfg(feature = "application-ipc")]
mod ipc_wake;
mod launch;
mod menu;
mod navigation;
mod options;
mod response;
mod scheme;
mod state;
mod tasks;
pub(crate) mod theme;
mod tray;

/// Scheme and authority the macOS backend serves app content from, with no
/// trailing slash. WKWebView registers `webui` as a custom scheme handler.
pub(crate) const APP_ORIGIN: &str = "webui://app";
mod window;

// Exercise the production Windows attachment invariant with the real command
// channel on macOS, without a Windows runtime or a native window.
#[cfg(test)]
#[path = "windows/wakeup.rs"]
mod windows_wakeup_contract;

#[cfg(all(test, feature = "application-ipc"))]
#[path = "linux/ipc_input.rs"]
mod gtk_input_contract;

use app_delegate::DesktopAppDelegate;

/// Options threaded from a [`DesktopFrame`] into [`DesktopAppDelegate::new`].
struct MacosLaunchOptions {
    executor: Arc<crate::execution::ApplicationExecutor>,
    runtime: Option<Arc<DesktopRuntime>>,
    #[cfg(feature = "local-server")]
    local_origin: Option<crate::LoopbackOrigin>,
    #[cfg(feature = "local-server")]
    local_url: Option<String>,
    #[cfg(feature = "local-server")]
    lifetime: Option<crate::HostLifetime>,
    #[cfg(feature = "local-server")]
    frame_policy: Option<Arc<crate::frame_policy::FramePolicy>>,
    #[cfg(feature = "local-server")]
    url_activation: Option<Arc<crate::local_server::url_activation::ActivationSender>>,
    #[cfg(feature = "native-services")]
    native_services: Option<crate::NativeServices>,
    live_background: Arc<crate::window::LiveBackground>,
    #[cfg(feature = "application-ipc")]
    ipc: Option<std::rc::Rc<ipc::MacIpc>>,
    title: Retained<NSString>,
    options: WindowOptions,
    shell: DesktopShellConfig,
    events: EventRegistry,
    window_handle: crate::WindowHandle,
    state_store: Option<WindowStateStore>,
}

/// Run a packaged desktop app bundle.
///
/// # Errors
///
/// Returns an error if packaged resources cannot be found or AppKit cannot
/// initialize.
pub fn run_packaged_app() -> Result<()> {
    crate::run_packaged_app().map_err(Into::into)
}

/// Run a prebuilt desktop runtime in a macOS WKWebView.
///
/// # Errors
///
/// Returns an error if AppKit cannot start on the main thread.
pub fn run_runtime(runtime: Arc<DesktopRuntime>, window: crate::WindowOptions) -> Result<()> {
    crate::run_runtime(runtime, window).map_err(Into::into)
}

pub(crate) fn run_frame(frame: DesktopFrame) -> Result<()> {
    let mtm = MainThreadMarker::new().context("macOS desktop must run on the main thread")?;
    let state_store =
        WindowStateStore::for_window(frame.window.remember_state, frame.app_id.as_deref())?;
    let window = frame.window.clone();
    let options = MacosLaunchOptions {
        executor: Arc::clone(&frame.executor),
        runtime: Some(Arc::clone(&frame.runtime)),
        #[cfg(feature = "local-server")]
        local_origin: None,
        #[cfg(feature = "local-server")]
        local_url: None,
        #[cfg(feature = "local-server")]
        lifetime: None,
        #[cfg(feature = "local-server")]
        frame_policy: None,
        #[cfg(feature = "local-server")]
        url_activation: None,
        #[cfg(feature = "native-services")]
        native_services: None,
        live_background: frame.runtime.live_background(),
        #[cfg(feature = "application-ipc")]
        ipc: frame
            .ipc_bridge()
            .is_enabled()
            .then(|| ipc::MacIpc::new(frame.ipc_bridge())),
        title: NSString::from_str(&window.title),
        options: window,
        shell: frame.shell.clone(),
        events: frame.events.clone(),
        window_handle: frame.window_handle.clone(),
        state_store,
    };
    run_app(mtm, options)
}

#[cfg(feature = "local-server")]
pub(crate) fn run_local_server_frame(frame: crate::LocalServerFrame) -> Result<()> {
    frame.lifetime().require_active()?;
    let mtm = MainThreadMarker::new().context("macOS desktop must run on the main thread")?;
    let state_store =
        WindowStateStore::for_window(frame.window.remember_state, frame.app_id.as_deref())?;
    let options = MacosLaunchOptions {
        executor: Arc::clone(&frame.executor),
        runtime: None,
        local_origin: Some(frame.origin().clone()),
        local_url: Some(frame.options.url()),
        lifetime: Some(frame.lifetime().clone()),
        frame_policy: Some(Arc::clone(&frame.frame_policy)),
        url_activation: frame
            .url_activation
            .lock()
            .map_err(|_| anyhow::anyhow!("URL activation registration unavailable"))?
            .as_ref()
            .map(Arc::clone),
        #[cfg(feature = "native-services")]
        native_services: frame
            .native_services
            .lock()
            .ok()
            .and_then(|services| services.as_ref().cloned()),
        live_background: Arc::clone(&frame.live_background),
        #[cfg(feature = "application-ipc")]
        ipc: frame.ipc_bridge().map(|bridge| {
            ipc::MacIpc::new_local(bridge, frame.origin().clone(), frame.lifetime().clone())
        }),
        title: NSString::from_str(&frame.window.title),
        options: frame.window.clone(),
        shell: frame.shell.clone(),
        events: frame.events.clone(),
        window_handle: frame.window_handle.clone(),
        state_store,
    };
    run_app(mtm, options)
}

fn run_app(mtm: MainThreadMarker, options: MacosLaunchOptions) -> Result<()> {
    autoreleasepool(|_| {
        let (app, delegate) = autoreleasepool(|_| {
            let app = NSApplication::sharedApplication(mtm);
            let delegate = DesktopAppDelegate::new(mtm, options);
            app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            (app, delegate)
        });
        app.run();
        #[cfg(feature = "local-server")]
        let unexpected_local_stop = delegate.ivars().local_origin.is_some()
            && !delegate.ivars().exiting.get()
            // Startup abort explicitly detaches the window delegate and
            // closes that window before stopping AppKit. A recorded Quit
            // error does not: its live window retains this delegate, even
            // when windowShouldClose vetoed the fallback close.
            && delegate
                .ivars()
                .window
                .get()
                .is_some_and(|window| window.delegate().is_some());
        #[cfg(feature = "local-server")]
        if unexpected_local_stop {
            // NSApplication::stop: can return without windowWillClose. Do not
            // retire IPC, unregister the owner close wake, emit Exiting or
            // release the frame's listener pin while its HTTP window lives.
            // The AppKit run loop has already stopped, so this is a terminal
            // fallback rather than a cancellable user close request.
            if let Some(window) = delegate.ivars().window.get() {
                window.close();
                if !delegate.ivars().exiting.get() {
                    eprintln!("WebUI: AppKit stopped without acknowledging local-server window close; waiting for native teardown before releasing the listener pin");
                }
                while !delegate.ivars().exiting.get() {
                    app.run();
                    if !delegate.ivars().exiting.get() {
                        window.close();
                    }
                }
            }
        }
        #[cfg(feature = "local-server")]
        delegate.cancel_quit_deadline();
        #[cfg(feature = "local-server")]
        if let Some(sender) = &delegate.ivars().url_activation {
            sender.close();
        }
        #[cfg(feature = "local-server")]
        let window_closed = delegate.ivars().exiting.get();
        if let Some(wake) = delegate.ivars().command_wake.get() {
            wake.close();
        }
        #[cfg(feature = "local-server")]
        {
            if let Some(wake) = delegate.ivars().owner_close_wake.get() {
                wake.close();
            }
            delegate
                .ivars()
                .owner_close_registration
                .borrow_mut()
                .take();
        }
        #[cfg(feature = "application-ipc")]
        if let Some(ipc) = &delegate.ivars().ipc {
            ipc.close();
        }
        if !delegate.ivars().exiting.replace(true) {
            let _ = delegate.ivars().events.dispatch(&DesktopEvent::Exiting);
        }
        #[cfg(feature = "local-server")]
        if let Some(error) = delegate.ivars().startup_error.borrow_mut().take() {
            app.setDelegate(None);
            app.setMainMenu(None);
            return Err(error.into());
        }
        #[cfg(feature = "local-server")]
        if unexpected_local_stop {
            app.setDelegate(None);
            app.setMainMenu(None);
            anyhow::bail!(
                "local-server AppKit loop stopped before WindowClosed; native window was closed and IPC retired before returning"
            );
        }
        #[cfg(feature = "local-server")]
        if delegate.ivars().local_origin.is_some() && delegate.ivars().window.get().is_none() {
            app.setDelegate(None);
            app.setMainMenu(None);
            anyhow::bail!("local-server AppKit loop returned without constructing a window");
        }
        #[cfg(feature = "local-server")]
        if delegate.ivars().local_origin.is_some() && !window_closed {
            app.setDelegate(None);
            app.setMainMenu(None);
            anyhow::bail!("local-server AppKit loop returned without WindowClosed");
        }
        #[cfg(feature = "local-server")]
        if delegate.ivars().local_origin.is_some() {
            app.setDelegate(None);
            app.setMainMenu(None);
        }
        Ok(())
    })
}

#[cfg(feature = "local-server")]
fn stop_local_app(app: &NSApplication) {
    // An AppKit `stop:` requested from a GCD command callback (rather than
    // from a dequeued NSEvent) can leave `run` waiting for its next event.
    // Wake the native event pump so the Rust host actually regains control.
    app.stop(None);
    if let Some(event) =
        NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
            NSEventType::ApplicationDefined,
            objc2_foundation::NSPoint::new(0.0, 0.0),
            NSEventModifierFlags::empty(),
            0.0,
            0,
            None,
            0,
            0,
            0,
        )
    {
        app.postEvent_atStart(&event, true);
    } else {
        eprintln!("WebUI: AppKit could not create the local-server run-loop wake event");
    }
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
    let mut url = String::with_capacity(APP_ORIGIN.len() + path.len());
    url.push_str(APP_ORIGIN);
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

/// Dispatch a lifecycle event to Rust callbacks and mirror it to the JS side.
pub(crate) fn dispatch_event(
    events: &EventRegistry,
    webview: &WKWebView,
    event: DesktopEvent,
) -> EventResponse {
    let response = events.dispatch(&event);
    if let Ok(script) = event.to_javascript() {
        // SAFETY: WebKit evaluates this owned event-dispatch script on its main-thread web view.
        unsafe { webview.evaluateJavaScript_completionHandler(&NSString::from_str(&script), None) };
    }
    response
}
