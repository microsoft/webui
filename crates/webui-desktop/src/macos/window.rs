// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! The `NSWindow` subclass and per-window option application.
//!
//! `DesktopWindow` overrides `canBecomeKeyWindow`/`canBecomeMainWindow` so
//! fully borderless (`TitlebarStyle::None`) windows still accept keyboard
//! input; plain `NSWindow` refuses key/main status for borderless windows.

use crate::WindowOptions;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, MainThreadOnly};
use objc2_app_kit::{NSWindow, NSWindowDelegate, NSWindowTitleVisibility};
use objc2_foundation::{NSObjectProtocol, NSSize};

use super::options::native_window_style;

define_class!(
    // SAFETY: NSWindow allows subclasses that do not add Drop behavior.
    #[unsafe(super = NSWindow)]
    #[thread_kind = MainThreadOnly]
    pub(super) struct DesktopWindow;

    impl DesktopWindow {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool {
            true
        }

        #[unsafe(method(performClose:))]
        fn perform_close(&self, sender: Option<&AnyObject>) {
            if self
                .styleMask()
                .contains(objc2_app_kit::NSWindowStyleMask::Titled)
            {
                // SAFETY: Call NSWindow's implementation, not this override,
                // with the original action sender on the owning UI thread.
                unsafe {
                    let _: () = msg_send![super(self), performClose: sender];
                }
                return;
            }
            // AppKit's performClose requires a native close button. Frameless
            // windows still consult the same delegate before closing.
            if let Some(delegate) = self.delegate() {
                if delegate.respondsToSelector(sel!(windowShouldClose:))
                    && !delegate.windowShouldClose(self)
                {
                    return;
                }
            }
            self.close();
        }
    }
);

/// Apply size constraints, titlebar transparency, and always-on-top level.
pub(super) fn apply_window_options(window: &NSWindow, options: &WindowOptions) {
    let style = native_window_style(options);
    if style.transparent_titlebar {
        window.setTitlebarAppearsTransparent(true);
        window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
    }
    if let (Some(width), Some(height)) = (options.min_width, options.min_height) {
        window.setContentMinSize(NSSize::new(f64::from(width), f64::from(height)));
    }
    if let (Some(width), Some(height)) = (options.max_width, options.max_height) {
        window.setContentMaxSize(NSSize::new(f64::from(width), f64::from(height)));
    }
    if options.always_on_top {
        // SAFETY: NSFloatingWindowLevel is the documented AppKit level value.
        unsafe {
            let _: () = objc2::msg_send![window, setLevel: 3_i64];
        };
    }
}
