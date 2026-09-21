// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Pure Windows lifecycle-transition and DPI-coordinate policy.

use crate::DesktopEvent;
use windows::Win32::UI::WindowsAndMessaging;

use super::state::WindowSizeState;
use super::WINDOW_ID;

/// Return the state and optional lifecycle event for one `WM_SIZE` code.
pub(super) fn size_event_transition(
    previous: WindowSizeState,
    size_code: u32,
) -> (WindowSizeState, Option<DesktopEvent>) {
    let current = match size_code {
        WindowsAndMessaging::SIZE_MAXIMIZED => WindowSizeState::Maximized,
        WindowsAndMessaging::SIZE_MINIMIZED => WindowSizeState::Minimized,
        _ => WindowSizeState::Normal,
    };
    let event = match (previous, current) {
        (WindowSizeState::Normal, WindowSizeState::Maximized)
        | (WindowSizeState::Minimized, WindowSizeState::Maximized) => {
            Some(DesktopEvent::WindowMaximized {
                window_id: WINDOW_ID,
            })
        }
        (WindowSizeState::Normal, WindowSizeState::Minimized)
        | (WindowSizeState::Maximized, WindowSizeState::Minimized) => {
            Some(DesktopEvent::WindowMinimized {
                window_id: WINDOW_ID,
            })
        }
        (WindowSizeState::Maximized, WindowSizeState::Normal) => {
            Some(DesktopEvent::WindowUnmaximized {
                window_id: WINDOW_ID,
            })
        }
        (WindowSizeState::Minimized, WindowSizeState::Normal) => {
            Some(DesktopEvent::WindowRestored {
                window_id: WINDOW_ID,
            })
        }
        _ => None,
    };
    (current, event)
}

/// Convert physical pixels to logical pixels, rounded to the nearest pixel.
pub(super) fn physical_to_logical(value: i32, dpi: u32) -> i32 {
    let divisor = i64::from(if dpi == 0 { 96 } else { dpi });
    let numerator = i64::from(value) * 96;
    let rounded = if numerator.is_negative() {
        (numerator - divisor / 2) / divisor
    } else {
        (numerator + divisor / 2) / divisor
    };
    i32::try_from(rounded).unwrap_or(if rounded.is_negative() {
        i32::MIN
    } else {
        i32::MAX
    })
}

/// Convert a non-negative physical client dimension to logical pixels.
pub(super) fn logical_dimension(value: i32, dpi: u32) -> u32 {
    u32::try_from(physical_to_logical(value, dpi)).unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn size_transitions_emit_only_on_state_changes() {
        assert_eq!(
            size_event_transition(WindowSizeState::Normal, WindowsAndMessaging::SIZE_MAXIMIZED),
            (
                WindowSizeState::Maximized,
                Some(DesktopEvent::WindowMaximized {
                    window_id: WINDOW_ID
                })
            )
        );
        assert_eq!(
            size_event_transition(
                WindowSizeState::Maximized,
                WindowsAndMessaging::SIZE_RESTORED
            ),
            (
                WindowSizeState::Normal,
                Some(DesktopEvent::WindowUnmaximized {
                    window_id: WINDOW_ID
                })
            )
        );
        assert_eq!(
            size_event_transition(WindowSizeState::Normal, WindowsAndMessaging::SIZE_MINIMIZED),
            (
                WindowSizeState::Minimized,
                Some(DesktopEvent::WindowMinimized {
                    window_id: WINDOW_ID
                })
            )
        );
        assert_eq!(
            size_event_transition(
                WindowSizeState::Minimized,
                WindowsAndMessaging::SIZE_RESTORED
            ),
            (
                WindowSizeState::Normal,
                Some(DesktopEvent::WindowRestored {
                    window_id: WINDOW_ID
                })
            )
        );
        assert_eq!(
            size_event_transition(WindowSizeState::Normal, WindowsAndMessaging::SIZE_RESTORED),
            (WindowSizeState::Normal, None)
        );
        assert_eq!(
            size_event_transition(
                WindowSizeState::Maximized,
                WindowsAndMessaging::SIZE_MAXIMIZED
            ),
            (WindowSizeState::Maximized, None)
        );
    }

    #[test]
    fn physical_coordinates_use_logical_pixels_and_safe_zero_dpi() {
        assert_eq!(physical_to_logical(1200, 144), 800);
        assert_eq!(physical_to_logical(-300, 144), -200);
        assert_eq!(physical_to_logical(1, 144), 1);
        assert_eq!(physical_to_logical(1200, 0), 1200);
        assert_eq!(logical_dimension(1200, 144), 800);
    }
}
