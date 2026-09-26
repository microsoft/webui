// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

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
