// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Window-geometry persistence across launches (`remember_state`).

use crate::{DisplayBounds, WindowState};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSScreen, NSWindow};
use objc2_foundation::{NSPoint, NSRect, NSSize};

use super::geometry::{clamp_coordinate, clamp_dimension};

/// Collect the current display work areas used to validate saved geometry.
#[must_use]
pub(super) fn display_bounds(mtm: MainThreadMarker) -> Vec<DisplayBounds> {
    NSScreen::screens(mtm)
        .to_vec()
        .iter()
        .map(|screen| {
            let frame = screen.frame();
            DisplayBounds {
                x: clamp_coordinate(frame.origin.x),
                y: clamp_coordinate(frame.origin.y),
                width: clamp_dimension(frame.size.width),
                height: clamp_dimension(frame.size.height),
            }
        })
        .collect()
}

/// Capture the window's current geometry for persistence.
#[must_use]
pub(super) fn capture_state(window: &NSWindow) -> WindowState {
    let frame = window.frame();
    WindowState {
        x: clamp_coordinate(frame.origin.x),
        y: clamp_coordinate(frame.origin.y),
        width: clamp_dimension(frame.size.width),
        height: clamp_dimension(frame.size.height),
        maximized: window.isZoomed(),
    }
}

/// Apply previously validated geometry to a not-yet-shown window.
pub(super) fn apply_state(window: &NSWindow, state: &WindowState) {
    let rect = NSRect::new(
        NSPoint::new(f64::from(state.x), f64::from(state.y)),
        NSSize::new(f64::from(state.width), f64::from(state.height)),
    );
    window.setFrame_display(rect, false);
    if state.maximized && !window.isZoomed() {
        window.zoom(None);
    }
}
