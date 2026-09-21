// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Custom non-client frame geometry: `WM_NCCALCSIZE` client extension,
//! `WM_NCHITTEST` resize borders and caption, and frame invalidation.

use webui_desktop::TitlebarStyle;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging;

use super::command::screen_point;
use super::state::with_window_state_result;

/// Thickness in pixels of the invisible resize border for custom frames.
const RESIZE_BORDER: i32 = 8;

/// Extend the client area into the frame for non-native titlebars.
///
/// A maximized window is positioned so its frame hangs off every edge of the
/// monitor. Returning the untouched rectangle would clip roughly a border width
/// of content per edge, so the maximized case re-inserts the frame thickness.
pub(super) fn non_client_calc_size(
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    let custom = with_window_state_result(hwnd, |state| {
        !matches!(state.titlebar(), TitlebarStyle::Native)
    })
    .unwrap_or(false);
    if !custom || w_param.0 == 0 || l_param.0 == 0 {
        // SAFETY: Native frames and the no-parameter form use default handling.
        return unsafe { WindowsAndMessaging::DefWindowProcW(hwnd, msg, w_param, l_param) };
    }
    // SAFETY: With a non-zero `w_param`, Windows passes a valid, writable
    // `NCCALCSIZE_PARAMS` in `l_param` for the duration of this message.
    unsafe {
        let params = &mut *(l_param.0 as *mut WindowsAndMessaging::NCCALCSIZE_PARAMS);
        if let Some(target) = params.rgrc.first_mut() {
            // SAFETY: `hwnd` is a live window; `IsZoomed` only reads state.
            if WindowsAndMessaging::IsZoomed(hwnd).as_bool() {
                let (frame_x, frame_y) = frame_thickness();
                target.left += frame_x;
                target.right -= frame_x;
                target.top += frame_y;
                target.bottom -= frame_y;
            }
        }
    }
    LRESULT(0)
}
/// Return the horizontal and vertical resize-frame thickness in pixels.
fn frame_thickness() -> (i32, i32) {
    // SAFETY: `GetSystemMetrics` reads process-wide, DPI-aware system values.
    unsafe {
        let padded = WindowsAndMessaging::GetSystemMetrics(WindowsAndMessaging::SM_CXPADDEDBORDER);
        (
            WindowsAndMessaging::GetSystemMetrics(WindowsAndMessaging::SM_CXFRAME) + padded,
            WindowsAndMessaging::GetSystemMetrics(WindowsAndMessaging::SM_CYFRAME) + padded,
        )
    }
}
/// Report resize borders, caption, and client regions for custom frames.
pub(super) fn non_client_hit_test(
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    let Some(titlebar) = with_window_state_result(hwnd, |state| state.titlebar().clone()) else {
        // SAFETY: Before state installation the system frame still applies.
        return unsafe { WindowsAndMessaging::DefWindowProcW(hwnd, msg, w_param, l_param) };
    };
    if matches!(titlebar, TitlebarStyle::Native) {
        // SAFETY: Native frames keep the system's own hit testing.
        return unsafe { WindowsAndMessaging::DefWindowProcW(hwnd, msg, w_param, l_param) };
    }
    let point = screen_point(l_param);
    let mut rect = RECT::default();
    // SAFETY: `hwnd` is a live window and `rect` is valid writable storage.
    if unsafe { WindowsAndMessaging::GetWindowRect(hwnd, &mut rect) }.is_err() {
        return LRESULT(hit_result(WindowsAndMessaging::HTCLIENT));
    }
    // SAFETY: `hwnd` is a live window; `IsZoomed` only reads window state.
    let zoomed = unsafe { WindowsAndMessaging::IsZoomed(hwnd) }.as_bool();
    if !zoomed {
        if let Some(border) = resize_hit(point, rect) {
            return LRESULT(hit_result(border));
        }
    }
    if point.y < rect.top + caption_height(&titlebar) {
        return LRESULT(hit_result(WindowsAndMessaging::HTCAPTION));
    }
    LRESULT(hit_result(WindowsAndMessaging::HTCLIENT))
}
/// Return the draggable caption height in physical pixels.
fn caption_height(titlebar: &TitlebarStyle) -> i32 {
    match titlebar {
        TitlebarStyle::Overlay { height } => i32::try_from(*height).unwrap_or(32),
        TitlebarStyle::HiddenInset => 28,
        TitlebarStyle::Native | TitlebarStyle::None => 0,
    }
}
/// Map a screen point onto one of the eight resize borders.
fn resize_hit(point: POINT, rect: RECT) -> Option<u32> {
    let left = point.x < rect.left + RESIZE_BORDER;
    let right = point.x >= rect.right - RESIZE_BORDER;
    let top = point.y < rect.top + RESIZE_BORDER;
    let bottom = point.y >= rect.bottom - RESIZE_BORDER;
    match (top, bottom, left, right) {
        (true, _, true, _) => Some(WindowsAndMessaging::HTTOPLEFT),
        (true, _, _, true) => Some(WindowsAndMessaging::HTTOPRIGHT),
        (_, true, true, _) => Some(WindowsAndMessaging::HTBOTTOMLEFT),
        (_, true, _, true) => Some(WindowsAndMessaging::HTBOTTOMRIGHT),
        (true, ..) => Some(WindowsAndMessaging::HTTOP),
        (_, true, ..) => Some(WindowsAndMessaging::HTBOTTOM),
        (_, _, true, _) => Some(WindowsAndMessaging::HTLEFT),
        (_, _, _, true) => Some(WindowsAndMessaging::HTRIGHT),
        _ => None,
    }
}
/// Convert a hit-test constant into a window-procedure result.
fn hit_result(value: u32) -> isize {
    isize::try_from(value).unwrap_or(1)
}
/// Recompute the frame after a desktop composition change.
pub(super) fn redraw_frame(hwnd: HWND) {
    // SAFETY: `hwnd` is a live window and only its frame is invalidated.
    unsafe {
        let _ = WindowsAndMessaging::SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            WindowsAndMessaging::SWP_NOMOVE
                | WindowsAndMessaging::SWP_NOSIZE
                | WindowsAndMessaging::SWP_NOZORDER
                | WindowsAndMessaging::SWP_FRAMECHANGED,
        );
    }
}
#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    const RECT_100: RECT = RECT {
        left: 0,
        top: 0,
        right: 100,
        bottom: 100,
    };

    #[test]
    fn resize_hit_reports_every_border_and_corner() {
        let at = |x, y| resize_hit(POINT { x, y }, RECT_100);
        assert_eq!(at(0, 0), Some(WindowsAndMessaging::HTTOPLEFT));
        assert_eq!(at(99, 0), Some(WindowsAndMessaging::HTTOPRIGHT));
        assert_eq!(at(0, 99), Some(WindowsAndMessaging::HTBOTTOMLEFT));
        assert_eq!(at(99, 99), Some(WindowsAndMessaging::HTBOTTOMRIGHT));
        assert_eq!(at(50, 0), Some(WindowsAndMessaging::HTTOP));
        assert_eq!(at(50, 99), Some(WindowsAndMessaging::HTBOTTOM));
        assert_eq!(at(0, 50), Some(WindowsAndMessaging::HTLEFT));
        assert_eq!(at(99, 50), Some(WindowsAndMessaging::HTRIGHT));
        assert_eq!(at(50, 50), None);
    }

    #[test]
    fn caption_height_tracks_requested_titlebar() {
        assert_eq!(caption_height(&TitlebarStyle::Overlay { height: 40 }), 40);
        assert_eq!(caption_height(&TitlebarStyle::HiddenInset), 28);
        assert_eq!(caption_height(&TitlebarStyle::None), 0);
        assert_eq!(caption_height(&TitlebarStyle::Native), 0);
    }
}
