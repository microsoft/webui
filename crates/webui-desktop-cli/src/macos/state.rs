// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Window-geometry persistence across launches (`remember_state`).

use objc2::MainThreadMarker;
use objc2_app_kit::{NSScreen, NSWindow};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use webui_desktop::{DisplayBounds, WindowState, WindowStateStore};

use super::geometry::{clamp_coordinate, clamp_dimension};

/// Derive a filesystem-stable state-store identifier from the window title.
///
/// The desktop shell configuration does not carry a stable app id through to
/// this backend, so the sanitized window title is the best available proxy:
/// it is deterministic per app and stable across launches of the same app.
#[must_use]
pub(super) fn state_app_id(title: &str) -> String {
    let mut id = String::with_capacity(title.len());
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            id.push(ch.to_ascii_lowercase());
        } else if !id.ends_with('-') {
            id.push('-');
        }
    }
    let trimmed = id.trim_matches('-');
    if trimmed.is_empty() {
        "webui-desktop-app".to_string()
    } else {
        format!("webui-desktop-{trimmed}")
    }
}

/// Build the state store for a window title.
#[must_use]
pub(super) fn state_store(title: &str) -> WindowStateStore {
    WindowStateStore::for_app_id(&state_app_id(title))
}

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

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn state_app_id_sanitizes_and_lowercases() {
        assert_eq!(
            state_app_id("Contact Book Manager"),
            "webui-desktop-contact-book-manager"
        );
        assert_eq!(state_app_id("  "), "webui-desktop-app");
        assert_eq!(state_app_id(""), "webui-desktop-app");
    }

    #[test]
    fn state_app_id_is_stable_for_repeated_calls() {
        assert_eq!(state_app_id("My App!!"), state_app_id("My App!!"));
    }
}
