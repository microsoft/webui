// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! The `NSWindow` subclass and per-window option application.
//!
//! `DesktopWindow` overrides `canBecomeKeyWindow`/`canBecomeMainWindow` so
//! fully borderless (`TitlebarStyle::None`) windows still accept keyboard
//! input; plain `NSWindow` refuses key/main status for borderless windows.

use crate::WindowOptions;
use objc2::{define_class, MainThreadOnly};
use objc2_app_kit::{NSWindow, NSWindowTitleVisibility};
use objc2_foundation::NSSize;

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
