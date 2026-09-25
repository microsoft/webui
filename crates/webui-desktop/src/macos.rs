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
use objc2_foundation::{MainThreadMarker, NSString};
use objc2_web_kit::WKWebView;

mod app_delegate;
mod commands;
mod effects;
mod geometry;
mod host_message;
mod icon;
#[cfg(feature = "application-ipc")]
mod ipc;
#[cfg(feature = "application-ipc")]
mod ipc_control;
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
mod theme;
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
    runtime: Arc<DesktopRuntime>,
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
    run_app(mtm, frame, state_store)
}

fn run_app(
    mtm: MainThreadMarker,
    frame: DesktopFrame,
    state_store: Option<WindowStateStore>,
) -> Result<()> {
    let window = frame.window.clone();
    autoreleasepool(|_| {
        let (app, delegate) = autoreleasepool(|_| {
            let app = NSApplication::sharedApplication(mtm);
            let title = NSString::from_str(&window.title);
            let delegate = DesktopAppDelegate::new(
                mtm,
                MacosLaunchOptions {
                    executor: Arc::clone(&frame.executor),
                    runtime: Arc::clone(&frame.runtime),
                    #[cfg(feature = "application-ipc")]
                    ipc: frame
                        .ipc_bridge()
                        .is_enabled()
                        .then(|| ipc::MacIpc::new(frame.ipc_bridge())),
                    title,
                    options: window,
                    shell: frame.shell.clone(),
                    events: frame.events.clone(),
                    window_handle: frame.window_handle.clone(),
                    state_store,
                },
            );
            app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            (app, delegate)
        });
        app.run();
        if let Some(wake) = delegate.ivars().command_wake.get() {
            wake.close();
        }
        #[cfg(feature = "application-ipc")]
        if let Some(ipc) = &delegate.ivars().ipc {
            ipc.close();
        }
        if !delegate.ivars().exiting.replace(true) {
            let _ = frame.events.dispatch(&DesktopEvent::Exiting);
        }
        // Keep the owning frame alive until AppKit and its callbacks finish.
        Ok(())
    })
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
fn dispatch_event(
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
