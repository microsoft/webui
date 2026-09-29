// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "local-server")]
use std::sync::Arc;

use crate::window::live_background_script;
use crate::{Rgba, WindowCommand, WindowHandle};
use block2::RcBlock;
use objc2::rc::Weak;
use objc2::runtime::AnyObject;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSWindow};
use objc2_foundation::{NSError, NSString};
use objc2_web_kit::WKWebView;

use super::effects::apply_background;

thread_local! {
    static TARGETS: RefCell<HashMap<u64, Rc<dyn Fn()>>> = RefCell::new(HashMap::new());
}
static NEXT_TARGET: AtomicU64 = AtomicU64::new(1);

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;

pub(super) struct CommandWake {
    id: u64,
    closed: Cell<bool>,
    _ui_local: std::marker::PhantomData<Rc<()>>,
}

impl CommandWake {
    pub(super) fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        let target = TARGETS.with(|targets| targets.borrow_mut().remove(&self.id));
        drop(target);
    }
}

impl Drop for CommandWake {
    fn drop(&mut self) {
        self.close();
    }
}

fn register_target(drain: Rc<dyn Fn()>) -> CommandWake {
    let id = NEXT_TARGET.fetch_add(1, Ordering::Relaxed);
    TARGETS.with(|targets| targets.borrow_mut().insert(id, drain));
    CommandWake {
        id,
        closed: Cell::new(false),
        _ui_local: std::marker::PhantomData,
    }
}

// `dispatch_get_main_queue()` is a header-only inline wrapper around the
// `_dispatch_main_q` symbol in libdispatch (part of `libSystem`), so the
// queue itself - not a `dispatch_get_main_queue` function - is what actually
// exists to link against.
#[link(name = "System")]
unsafe extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_async_f(
        queue: *mut c_void,
        context: *mut c_void,
        work: unsafe extern "C" fn(*mut c_void),
    );
}

fn dispatch_get_main_queue() -> *mut c_void {
    // `_dispatch_main_q` is a statically allocated libdispatch queue that lives
    // for the process lifetime; taking its address never dereferences it, so
    // this is safe even though the static itself is `unsafe extern`.
    std::ptr::addr_of!(_dispatch_main_q).cast_mut()
}

pub(super) fn install_wakeup(
    window: &NSWindow,
    webview: &WKWebView,
    handle: &WindowHandle,
) -> CommandWake {
    let window = Weak::new(window);
    let webview = Weak::new(webview);
    let commands = handle.clone();
    let target = register_target(Rc::new(move || {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some(window) = window.load() else {
            return;
        };
        let Some(webview) = webview.load() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        for command in commands.drain_commands() {
            execute_command(&window, &webview, command, &app);
        }
    }));
    // Only an opaque id enters the Send + Sync wake callback. The owning
    // window is weak and UI-local; hidden/unfocused windows remain addressable,
    // and commands can never fall back to a different key/main window.
    let id = target.id;
    handle.set_wakeup(move || schedule_drain(id));
    target
}

fn schedule_drain(id: u64) {
    let context = Box::into_raw(Box::new(id)).cast::<c_void>();
    // SAFETY: `context` owns an id until `drain_on_main_queue` reconstructs
    // it. Grand Central Dispatch invokes that callback exactly once on the main queue.
    unsafe { dispatch_async_f(dispatch_get_main_queue(), context, drain_on_main_queue) };
}

#[cfg(feature = "local-server")]
fn owner_close_callback(
    #[cfg(feature = "native-capture")] capture: Option<Arc<crate::capture::CaptureState>>,
    #[cfg(feature = "native-clipboard")] clipboard: Option<Arc<crate::clipboard::ClipboardState>>,
    schedule: impl Fn() + Send + Sync + 'static,
) -> Arc<dyn Fn() + Send + Sync> {
    Arc::new(move || {
        #[cfg(feature = "native-capture")]
        if let Some(capture) = &capture {
            // This runs synchronously under HostLifetime's close lock. Never
            // invoke a Rust Future waker or wait for AppKit here.
            capture.close();
        }
        #[cfg(feature = "native-clipboard")]
        if let Some(clipboard) = &clipboard {
            clipboard.close_silent();
        }
        schedule();
    })
}

#[cfg(feature = "local-server")]
pub(super) fn install_owner_close(
    window: &NSWindow,
    lifetime: &crate::HostLifetime,
    #[cfg(feature = "native-capture")] capture: Option<Arc<crate::capture::CaptureState>>,
    #[cfg(feature = "native-clipboard")] clipboard: Option<Arc<crate::clipboard::ClipboardState>>,
) -> crate::Result<(CommandWake, crate::local_server::HostCloseRegistration)> {
    let window = Weak::new(window);
    let state = lifetime.clone();
    let target = register_target(Rc::new(move || {
        if !state.is_active() {
            if let Some(window) = window.load() {
                // Unlike performClose, this bypasses cancellable close
                // requests once the authenticated owner is gone.
                window.close();
            }
        }
    }));
    let id = target.id;
    let registration = lifetime.register_close(owner_close_callback(
        #[cfg(feature = "native-capture")]
        capture,
        #[cfg(feature = "native-clipboard")]
        clipboard,
        move || schedule_drain(id),
    ))?;
    Ok((target, registration))
}

unsafe extern "C" fn drain_on_main_queue(context: *mut c_void) {
    // SAFETY: `context` was created by `Box::into_raw` in `schedule_drain` and is
    // delivered exactly once by dispatch_async_f.
    let id = unsafe { Box::from_raw(context.cast::<u64>()) };
    drain_target(*id);
}

fn drain_target(id: u64) {
    let target = TARGETS.with(|targets| targets.borrow().get(&id).cloned());
    if let Some(target) = target {
        target();
    }
}

fn execute_command(
    window: &NSWindow,
    webview: &WKWebView,
    command: WindowCommand,
    app: &NSApplication,
) {
    match command {
        WindowCommand::SetTitle(title) => window.setTitle(&NSString::from_str(&title)),
        WindowCommand::SetBackground(color) => {
            apply_background(window, webview, Some(color));
            update_document_background(webview, color);
        }
        WindowCommand::SetSize { width, height } => window.setContentSize(
            objc2_foundation::NSSize::new(f64::from(width), f64::from(height)),
        ),
        WindowCommand::Minimize => window.miniaturize(None),
        WindowCommand::Maximize => window.zoom(None),
        WindowCommand::Unmaximize => {
            if window.isZoomed() {
                window.zoom(None);
            }
        }
        WindowCommand::SetFullscreen(enabled) => {
            if window
                .styleMask()
                .contains(objc2_app_kit::NSWindowStyleMask::FullScreen)
                != enabled
            {
                window.toggleFullScreen(None);
            }
        }
        WindowCommand::Center => window.center(),
        WindowCommand::Focus => window.makeKeyAndOrderFront(None),
        WindowCommand::Close => window.performClose(None),
        WindowCommand::StartDrag => {
            if let Some(event) = app.currentEvent() {
                window.performWindowDragWithEvent(&event);
            }
        }
        WindowCommand::SetAlwaysOnTop(value) => {
            // SAFETY: These are documented AppKit window-level constants and the live
            // NSWindow is accessed exclusively on the main thread.
            unsafe {
                let _: () = objc2::msg_send![window, setLevel: if value { 3_i64 } else { 0_i64 }];
            }
        }
    }
}

pub(super) fn update_document_background(webview: &WKWebView, color: Rgba) {
    let completion = RcBlock::new(|_: *mut AnyObject, error: *mut NSError| {
        if !error.is_null() {
            // SAFETY: WebKit keeps the error alive for this completion callback.
            let error = unsafe { &*error };
            eprintln!(
                "WebUI: failed to update the current document background: {}",
                error.localizedDescription()
            );
        }
    });
    // SAFETY: The live webview and completion are accessed on the main thread.
    unsafe {
        webview.evaluateJavaScript_completionHandler(
            &NSString::from_str(&live_background_script(color)),
            Some(&completion),
        );
    }
}

#[cfg(feature = "local-server")]
fn local_caption_insets_script(fullscreen: bool) -> &'static str {
    if fullscreen {
        "document.documentElement?.style.setProperty('--webui-titlebar-inset-start','0px')"
    } else {
        "document.documentElement?.style.setProperty('--webui-titlebar-inset-start','78px')"
    }
}

#[cfg(feature = "local-server")]
pub(super) fn update_local_caption_insets(webview: &WKWebView, fullscreen: bool) {
    let completion = RcBlock::new(|_: *mut AnyObject, error: *mut NSError| {
        if !error.is_null() {
            // SAFETY: WebKit keeps the error alive for this completion callback.
            let error = unsafe { &*error };
            eprintln!(
                "WebUI: failed to update native caption insets: {}",
                error.localizedDescription()
            );
        }
    });
    // SAFETY: This runs on the AppKit thread against the live main-frame view.
    unsafe {
        webview.evaluateJavaScript_completionHandler(
            &NSString::from_str(local_caption_insets_script(fullscreen)),
            Some(&completion),
        );
    }
}

#[cfg(all(test, feature = "local-server"))]
mod local_caption_tests {
    use super::local_caption_insets_script;

    #[test]
    fn fullscreen_releases_and_restores_native_caption_space() {
        assert!(local_caption_insets_script(true).contains("'0px'"));
        assert!(local_caption_insets_script(false).contains("'78px'"));
        assert!(local_caption_insets_script(true).contains("--webui-titlebar-inset-start"));
    }
}

#[cfg(all(test, feature = "native-clipboard"))]
#[allow(clippy::disallowed_methods)]
mod capture_close_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn composed_owner_revoke_retires_png_and_clipboard_before_wake() {
        let (owner, lifetime) = crate::HostLifetime::new();
        let capture = crate::capture::CaptureState::new(lifetime.clone(), 17);
        let clipboard =
            crate::clipboard::ClipboardState::new(lifetime.clone(), Arc::clone(&capture));
        capture.test_store_retained(25_000);
        assert_eq!(capture.test_retained_len(), 25_000);
        let queued = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&queued);
        let observed = Arc::clone(&capture);
        let during_wake = Arc::clone(&capture);
        let observed_clipboard = Arc::clone(&clipboard);
        let during_wake_clipboard = Arc::clone(&clipboard);
        let _registration = lifetime
            .register_close(owner_close_callback(
                Some(capture),
                Some(clipboard),
                move || {
                    assert_eq!(during_wake.test_retained_len(), 0);
                    assert!(during_wake_clipboard.test_is_closed());
                    counter.fetch_add(1, Ordering::AcqRel);
                },
            ))
            .unwrap();
        owner.revoke().unwrap();
        assert_eq!(observed.test_retained_len(), 0);
        assert!(observed_clipboard.test_is_closed());
        assert_eq!(queued.load(Ordering::Acquire), 1);
        owner.revoke().unwrap();
        assert_eq!(queued.load(Ordering::Acquire), 1);
        owner.retry_close().unwrap();
        assert_eq!(observed.test_retained_len(), 0);
        assert!(observed_clipboard.test_is_closed());
        assert_eq!(queued.load(Ordering::Acquire), 2);
    }
}

#[cfg(all(test, feature = "native-capture", not(feature = "native-clipboard")))]
#[allow(clippy::disallowed_methods)]
mod capture_only_close_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn capture_only_owner_revoke_discards_png_before_native_close_wake() {
        let (owner, lifetime) = crate::HostLifetime::new();
        let capture = crate::capture::CaptureState::new(lifetime.clone(), 18);
        capture.test_store_retained(25_000);
        let observed = Arc::clone(&capture);
        let wakes = Arc::new(AtomicUsize::new(0));
        let callback_wakes = Arc::clone(&wakes);
        let _registration = lifetime
            .register_close(owner_close_callback(Some(capture), move || {
                assert_eq!(observed.test_retained_len(), 0);
                callback_wakes.fetch_add(1, Ordering::AcqRel);
            }))
            .unwrap();
        owner.revoke().unwrap();
        assert_eq!(wakes.load(Ordering::Acquire), 1);
    }
}
