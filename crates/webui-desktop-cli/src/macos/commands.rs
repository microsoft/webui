// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::ffi::c_void;

use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use webui_desktop::{WindowCommand, WindowHandle};

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

pub(super) fn install_wakeup(handle: &WindowHandle) {
    let wake_handle = handle.clone();
    handle.set_wakeup(move || schedule_drain(wake_handle.clone()));
}

fn schedule_drain(handle: WindowHandle) {
    let context = Box::into_raw(Box::new(handle)).cast::<c_void>();
    // SAFETY: `context` owns a WindowHandle until `drain_on_main_queue` reconstructs
    // it. Grand Central Dispatch invokes that callback exactly once on the main queue.
    unsafe { dispatch_async_f(dispatch_get_main_queue(), context, drain_on_main_queue) };
}

unsafe extern "C" fn drain_on_main_queue(context: *mut c_void) {
    // SAFETY: `context` was created by `Box::into_raw` in `schedule_drain` and is
    // delivered exactly once by dispatch_async_f.
    let handle = unsafe { Box::from_raw(context.cast::<WindowHandle>()) };
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let Some(window) = app.keyWindow().or_else(|| app.mainWindow()) else {
        return;
    };
    for command in handle.drain_commands() {
        execute_command(&window, command, &app);
    }
}

fn execute_command(window: &objc2_app_kit::NSWindow, command: WindowCommand, app: &NSApplication) {
    match command {
        WindowCommand::SetTitle(title) => {
            window.setTitle(&objc2_foundation::NSString::from_str(&title))
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
