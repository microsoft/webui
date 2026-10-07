// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! The `NSWindow` subclass and per-window option application.
//!
//! `DesktopWindow` overrides `canBecomeKeyWindow`/`canBecomeMainWindow` so
//! fully borderless (`TitlebarStyle::None`) windows still accept keyboard
//! input; plain `NSWindow` refuses key/main status for borderless windows.

use crate::{TitlebarStyle, WindowOptions};
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, MainThreadOnly};
use objc2_app_kit::{
    NSWindow, NSWindowButton, NSWindowDelegate, NSWindowStyleMask, NSWindowTitleVisibility,
};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSSize};

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

fn caption_center_y(bounds: NSRect, flipped: bool, height: u32) -> Option<f64> {
    if !bounds.size.height.is_finite() || bounds.size.height <= 0.0 {
        return None;
    }
    let half_band = f64::from(height).min(bounds.size.height) / 2.0;
    Some(
        bounds.origin.y
            + if flipped {
                half_band
            } else {
                bounds.size.height - half_band
            },
    )
}

/// Keep AppKit's native buttons aligned with a full-height application header.
pub(super) fn align_overlay_controls(window: &NSWindow, options: &WindowOptions) {
    let TitlebarStyle::Overlay { height } = options.titlebar else {
        return;
    };
    if window.styleMask().contains(NSWindowStyleMask::FullScreen) {
        return;
    }
    let Some(content) = window.contentView() else {
        return;
    };
    let Some(center_y) = caption_center_y(content.bounds(), content.isFlipped(), height) else {
        return;
    };
    for kind in [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ] {
        let Some(button) = window.standardWindowButton(kind) else {
            continue;
        };
        // SAFETY: The live AppKit window retains the button hierarchy on its UI thread.
        let Some(parent) = (unsafe { button.superview() }) else {
            continue;
        };
        let frame = button.frame();
        let target = content.convertPoint_toView(NSPoint::new(0.0, center_y), Some(&parent));
        let next_y = target.y - frame.size.height / 2.0;
        if next_y.is_finite() && (frame.origin.y - next_y).abs() > 0.5 {
            button.setFrameOrigin(NSPoint::new(frame.origin.x, next_y));
        }
    }
}

#[cfg(test)]
mod overlay_tests {
    use super::*;

    #[test]
    fn header_center_uses_view_coordinates_and_clamps_to_visible_height() {
        let bounds = NSRect::new(NSPoint::new(0.0, 5.0), NSSize::new(1140.0, 1124.0));
        assert_eq!(caption_center_y(bounds, false, 64), Some(1097.0));
        assert_eq!(caption_center_y(bounds, true, 64), Some(37.0));
        assert_eq!(caption_center_y(bounds, false, 4000), Some(567.0));
        assert_eq!(caption_center_y(NSRect::ZERO, false, 64), None);
    }
}
