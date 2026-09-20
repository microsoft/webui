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
use anyhow::{Context, Result};
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::ProtocolObject;
use objc2_app_kit::NSApplication;
use objc2_foundation::{MainThreadMarker, NSString};
use objc2_web_kit::WKWebView;
use webui_desktop::{
    DesktopEvent, DesktopRuntime, DesktopShellConfig, EventRegistry, EventResponse, WindowOptions,
};

mod app_delegate;
mod commands;
mod effects;
mod geometry;
mod host_message;
mod launch;
mod menu;
mod navigation;
mod options;
mod response;
mod scheme;
mod state;
mod theme;
mod tray;

/// Scheme and authority the macOS backend serves app content from, with no
/// trailing slash. WKWebView registers `webui` as a custom scheme handler.
pub(crate) const APP_ORIGIN: &str = "webui://app";
mod window;

use app_delegate::DesktopAppDelegate;
use scheme::set_runtime;

/// Options threaded from a [`DesktopFrame`] into [`DesktopAppDelegate::new`].
struct MacosLaunchOptions {
    title: Retained<NSString>,
    options: WindowOptions,
    shell: DesktopShellConfig,
    events: EventRegistry,
    window_handle: webui_desktop::WindowHandle,
}

/// Run a packaged desktop app bundle.
///
/// # Errors
///
/// Returns an error if packaged resources cannot be found or AppKit cannot
/// initialize.
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
    set_runtime(Arc::clone(&frame.runtime));
    run_app(mtm, frame)
}

fn run_app(mtm: MainThreadMarker, frame: DesktopFrame) -> Result<()> {
    let window = frame.window;
    autoreleasepool(|_| {
        let (app, _delegate) = autoreleasepool(|_| {
            let app = NSApplication::sharedApplication(mtm);
            let title = NSString::from_str(&window.title);
            let delegate = DesktopAppDelegate::new(
                mtm,
                MacosLaunchOptions {
                    title,
                    options: window,
                    shell: frame.shell,
                    events: frame.events,
                    window_handle: frame.window_handle,
                },
            );
            app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            (app, delegate)
        });
        app.run();
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
