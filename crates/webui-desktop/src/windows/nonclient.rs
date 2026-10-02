// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Custom non-client frame geometry: `WM_NCCALCSIZE` client extension,
//! `WM_NCHITTEST` resize borders and caption, and frame invalidation.

use crate::{Rgba, TitlebarStyle, WindowOptions};
use windows::core::{Error, Result};
use windows::Win32::Foundation::{
    GetLastError, SetLastError, ERROR_SUCCESS, E_INVALIDARG, HWND, LPARAM, LRESULT, POINT, RECT,
    WPARAM,
};
use windows::Win32::Graphics::Dwm;
use windows::Win32::UI::{HiDpi, WindowsAndMessaging};

use super::command::screen_point;
use super::state::window_style_bits;

// Four Win32 LONGs keep creation-time frame layout, background, and startup
// lifecycle independent of WebView2 state.
pub(super) const CUSTOM_FRAME_BYTES: i32 = 16;
const CUSTOM_FRAME_INDEX: WindowsAndMessaging::WINDOW_LONG_PTR_INDEX =
    WindowsAndMessaging::WINDOW_LONG_PTR_INDEX(0);
const STARTUP_BACKGROUND_INDEX: WindowsAndMessaging::WINDOW_LONG_PTR_INDEX =
    WindowsAndMessaging::WINDOW_LONG_PTR_INDEX(4);
const STARTUP_CLOSE_INDEX: WindowsAndMessaging::WINDOW_LONG_PTR_INDEX =
    WindowsAndMessaging::WINDOW_LONG_PTR_INDEX(8);
const STARTUP_READY_INDEX: WindowsAndMessaging::WINDOW_LONG_PTR_INDEX =
    WindowsAndMessaging::WINDOW_LONG_PTR_INDEX(12);
const FRAME_NATIVE: i32 = 0;
pub(super) const FRAME_OVERLAY: i32 = 1;
const FRAME_NONE: i32 = 2;
const FRAME_WIN32_OVERLAY: i32 = 3;

pub(super) fn initialize_frame(hwnd: HWND, l_param: LPARAM) -> Result<()> {
    // SAFETY: WM_NCCREATE supplies CREATESTRUCTW for this synchronous call.
    let create = unsafe { (l_param.0 as *const WindowsAndMessaging::CREATESTRUCTW).as_ref() }
        .ok_or_else(|| Error::new(E_INVALIDARG, "missing native window creation parameters"))?;
    // SAFETY: FrameWindow::new passes a borrowed WindowOptions that remains
    // valid throughout CreateWindowExW. Only its frame mode is copied here.
    let options = unsafe { create.lpCreateParams.cast::<WindowOptions>().as_ref() }
        .ok_or_else(|| Error::new(E_INVALIDARG, "missing native window options"))?;
    let custom = match options.titlebar {
        TitlebarStyle::Native => FRAME_NATIVE,
        TitlebarStyle::None => FRAME_NONE,
        TitlebarStyle::HiddenInset | TitlebarStyle::Overlay { .. } => {
            if super::app_sdk::requires_runtime(options) {
                FRAME_OVERLAY
            } else {
                FRAME_WIN32_OVERLAY
            }
        }
    };
    // SAFETY: The registered window class reserves two LONGs at these indices.
    unsafe {
        SetLastError(ERROR_SUCCESS);
        WindowsAndMessaging::SetWindowLongW(hwnd, CUSTOM_FRAME_INDEX, custom);
        if GetLastError() != ERROR_SUCCESS {
            return Err(Error::from_thread());
        }
        WindowsAndMessaging::SetWindowLongW(
            hwnd,
            STARTUP_BACKGROUND_INDEX,
            encode_startup_background(options.background),
        );
        if GetLastError() != ERROR_SUCCESS {
            return Err(Error::from_thread());
        }
    }
    Ok(())
}

pub(super) fn startup_background(hwnd: HWND) -> Option<Rgba> {
    // SAFETY: WM_NCCREATE initializes this reserved LONG before the window can
    // be shown.
    let encoded = unsafe { WindowsAndMessaging::GetWindowLongW(hwnd, STARTUP_BACKGROUND_INDEX) };
    decode_startup_background(encoded)
}

pub(super) fn defer_startup_close(hwnd: HWND) {
    // SAFETY: The window class reserves this LONG for the startup-close flag.
    // Hiding the live window acknowledges the user's close immediately while
    // keeping its HWND valid for in-flight WebView2 callbacks.
    unsafe {
        WindowsAndMessaging::SetWindowLongW(hwnd, STARTUP_CLOSE_INDEX, 1);
        let _ = WindowsAndMessaging::ShowWindow(hwnd, WindowsAndMessaging::SW_HIDE);
    }
}

pub(super) fn mark_startup_ready(hwnd: HWND) -> bool {
    // SAFETY: The window class reserves both LONGs for startup lifecycle flags.
    unsafe {
        WindowsAndMessaging::SetWindowLongW(hwnd, STARTUP_READY_INDEX, 1);
        WindowsAndMessaging::GetWindowLongW(hwnd, STARTUP_CLOSE_INDEX) != 0
    }
}

pub(super) fn startup_is_ready(hwnd: HWND) -> bool {
    // SAFETY: The window class reserves this LONG for the startup-ready flag.
    unsafe { WindowsAndMessaging::GetWindowLongW(hwnd, STARTUP_READY_INDEX) != 0 }
}

fn encode_startup_background(color: Option<Rgba>) -> i32 {
    color.map_or(0, |color| {
        (i32::from(color.r) | (i32::from(color.g) << 8) | (i32::from(color.b) << 16)) + 1
    })
}

fn decode_startup_background(encoded: i32) -> Option<Rgba> {
    let rgb = u32::try_from(encoded).ok()?.checked_sub(1)?;
    Some(Rgba {
        r: u8::try_from(rgb & 0xff).ok()?,
        g: u8::try_from((rgb >> 8) & 0xff).ok()?,
        b: u8::try_from((rgb >> 16) & 0xff).ok()?,
        a: u8::MAX,
    })
}

/// Extend the client area into the frame for non-native titlebars.
///
/// Keep frameless resize edges outside WebView2. Maximized custom windows
/// also inset the frame that hangs off the monitor's work area.
pub(super) fn non_client_calc_size(
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    // SAFETY: WM_NCCREATE initialized the class's reserved frame-mode slot.
    let mode = unsafe { WindowsAndMessaging::GetWindowLongW(hwnd, CUSTOM_FRAME_INDEX) };
    if !matches!(mode, FRAME_NONE | FRAME_WIN32_OVERLAY) || l_param.0 == 0 {
        // SAFETY: Native frames and messages without geometry use default handling.
        return unsafe { WindowsAndMessaging::DefWindowProcW(hwnd, msg, w_param, l_param) };
    }
    // SAFETY: WM_NCCALCSIZE passes writable RECT storage when w_param is zero,
    // and NCCALCSIZE_PARAMS otherwise, for the duration of this message.
    unsafe {
        let target = if w_param.0 == 0 {
            &mut *(l_param.0 as *mut RECT)
        } else {
            &mut (*(l_param.0 as *mut WindowsAndMessaging::NCCALCSIZE_PARAMS)).rgrc[0]
        };
        let resizable = window_style_bits(hwnd, WindowsAndMessaging::GWL_STYLE)
            & WindowsAndMessaging::WS_THICKFRAME.0
            != 0;
        if resizable {
            let (frame_x, frame_y) = frame_thickness(hwnd);
            target.left += frame_x;
            target.right -= frame_x;
            target.bottom -= frame_y;
            if mode == FRAME_NONE || WindowsAndMessaging::IsZoomed(hwnd).as_bool() {
                target.top += frame_y;
            }
        }
    }
    LRESULT(0)
}
/// Return the horizontal and vertical resize-frame thickness in pixels.
pub(super) fn frame_thickness(hwnd: HWND) -> (i32, i32) {
    // SAFETY: These calls only read the live window's DPI and system metrics.
    unsafe {
        let dpi = HiDpi::GetDpiForWindow(hwnd);
        let metric = |index| HiDpi::GetSystemMetricsForDpi(index, dpi);
        let padded = metric(WindowsAndMessaging::SM_CXPADDEDBORDER);
        (
            metric(WindowsAndMessaging::SM_CXFRAME) + padded,
            metric(WindowsAndMessaging::SM_CYFRAME) + padded,
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
    // SAFETY: WM_NCCREATE initialized this reserved window slot.
    let mode = unsafe { WindowsAndMessaging::GetWindowLongW(hwnd, CUSTOM_FRAME_INDEX) };
    if mode == FRAME_WIN32_OVERLAY {
        let mut result = LRESULT(0);
        // SAFETY: DWM owns the native caption buttons for this live HWND and
        // writes one hit-test result synchronously when it handles the point.
        if unsafe { Dwm::DwmDefWindowProc(hwnd, msg, w_param, l_param, &mut result) }.as_bool() {
            return result;
        }
    } else if mode != FRAME_NONE {
        // SAFETY: Native frames keep the system's own hit testing.
        return unsafe { WindowsAndMessaging::DefWindowProcW(hwnd, msg, w_param, l_param) };
    }

    let style = window_style_bits(hwnd, WindowsAndMessaging::GWL_STYLE);
    if style & WindowsAndMessaging::WS_CAPTION.0 == 0 {
        return LRESULT(hit_result(WindowsAndMessaging::HTCLIENT));
    }
    let point = screen_point(l_param);
    let mut rect = RECT::default();
    // SAFETY: `hwnd` is a live window and `rect` is valid writable storage.
    if unsafe { WindowsAndMessaging::GetWindowRect(hwnd, &mut rect) }.is_err() {
        return LRESULT(hit_result(WindowsAndMessaging::HTCLIENT));
    }
    // SAFETY: `hwnd` is a live window; `IsZoomed` only reads window state.
    let zoomed = unsafe { WindowsAndMessaging::IsZoomed(hwnd) }.as_bool();
    if !zoomed && style & WindowsAndMessaging::WS_THICKFRAME.0 != 0 {
        if let Some(border) = resize_hit(point, rect, frame_thickness(hwnd)) {
            return LRESULT(hit_result(border));
        }
    }
    LRESULT(hit_result(WindowsAndMessaging::HTCLIENT))
}

pub(super) fn uses_win32_overlay(hwnd: HWND) -> bool {
    // SAFETY: WM_NCCREATE initializes the reserved frame-mode slot.
    unsafe { WindowsAndMessaging::GetWindowLongW(hwnd, CUSTOM_FRAME_INDEX) == FRAME_WIN32_OVERLAY }
}

/// Map a screen point onto one of the eight resize borders.
fn resize_hit(point: POINT, rect: RECT, (border_x, border_y): (i32, i32)) -> Option<u32> {
    let left = point.x < rect.left + border_x;
    let right = point.x >= rect.right - border_x;
    let top = point.y < rect.top + border_y;
    let bottom = point.y >= rect.bottom - border_y;
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
        let at = |x, y| resize_hit(POINT { x, y }, RECT_100, (8, 8));
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
    fn resize_hit_uses_per_axis_dpi_metrics() {
        assert_eq!(
            resize_hit(POINT { x: 11, y: 15 }, RECT_100, (12, 16)),
            Some(WindowsAndMessaging::HTTOPLEFT)
        );
        assert_eq!(resize_hit(POINT { x: 12, y: 16 }, RECT_100, (12, 16)), None);
        assert_eq!(
            resize_hit(POINT { x: 88, y: 84 }, RECT_100, (12, 16)),
            Some(WindowsAndMessaging::HTBOTTOMRIGHT)
        );
    }

    #[test]
    fn native_resize_hits_follow_live_window_capabilities_before_webview_setup() {
        use super::super::create::FrameWindow;

        {
            let titlebar = TitlebarStyle::None;
            for (resizable, maximized) in [(true, false), (false, false), (true, true)] {
                let frame = FrameWindow::new(
                    &WindowOptions {
                        titlebar: titlebar.clone(),
                        resizable,
                        maximized,
                        center: false,
                        ..WindowOptions::default()
                    },
                    None,
                )
                .unwrap();
                let mut rect = RECT::default();
                // SAFETY: The test owns the live window and writable rectangle.
                unsafe { WindowsAndMessaging::GetWindowRect(frame.hwnd, &mut rect).unwrap() };
                let hit = |x: i32, y: i32| {
                    let bits = ((y.cast_unsigned() & 0xffff) << 16) | (x.cast_unsigned() & 0xffff);
                    non_client_hit_test(
                        frame.hwnd,
                        WindowsAndMessaging::WM_NCHITTEST,
                        WPARAM(0),
                        LPARAM(isize::try_from(bits).unwrap()),
                    )
                };
                let expected = if resizable && !maximized {
                    WindowsAndMessaging::HTTOPLEFT
                } else {
                    WindowsAndMessaging::HTCLIENT
                };
                assert_eq!(hit(rect.left, rect.top).0, hit_result(expected));

                // Fullscreen removes the frame styles even when WS_MAXIMIZE
                // remains set. Neither geometry nor hits may retain a border.
                let style = window_style_bits(frame.hwnd, WindowsAndMessaging::GWL_STYLE)
                    & !WindowsAndMessaging::WS_OVERLAPPEDWINDOW.0;
                // SAFETY: The test owns this window and restores no retained state.
                unsafe {
                    WindowsAndMessaging::SetWindowLongW(
                        frame.hwnd,
                        WindowsAndMessaging::GWL_STYLE,
                        style.cast_signed(),
                    );
                }
                assert_eq!(
                    hit(rect.left, rect.top).0,
                    hit_result(WindowsAndMessaging::HTCLIENT)
                );
                let mut client = RECT_100;
                non_client_calc_size(
                    frame.hwnd,
                    WindowsAndMessaging::WM_NCCALCSIZE,
                    WPARAM(0),
                    LPARAM(std::ptr::from_mut(&mut client) as isize),
                );
                assert_eq!(client, RECT_100);
                // SAFETY: Destroy the test's window on the thread that created it.
                unsafe { WindowsAndMessaging::DestroyWindow(frame.hwnd).unwrap() };
            }
        }
    }

    #[test]
    fn sdk_tall_overlay_preserves_default_non_client_geometry_before_attachment() {
        let options = WindowOptions {
            titlebar: TitlebarStyle::Overlay { height: 48 },
            caption_button_size: crate::CaptionButtonSize::Tall,
            center: false,
            ..WindowOptions::default()
        };
        let frame = super::super::create::FrameWindow::new(&options, None).unwrap();
        let mut actual = RECT_100;
        let mut expected = RECT_100;
        // SAFETY: Each synchronous call borrows a distinct writable rectangle.
        let default_result = unsafe {
            WindowsAndMessaging::DefWindowProcW(
                frame.hwnd,
                WindowsAndMessaging::WM_NCCALCSIZE,
                WPARAM(0),
                LPARAM(std::ptr::from_mut(&mut expected) as isize),
            )
        };
        let result = non_client_calc_size(
            frame.hwnd,
            WindowsAndMessaging::WM_NCCALCSIZE,
            WPARAM(0),
            LPARAM(std::ptr::from_mut(&mut actual) as isize),
        );
        assert_eq!(result, default_result);
        assert_eq!(actual, expected);
    }

    #[test]
    fn startup_background_round_trips_rgb_and_uses_zero_for_none() {
        let color = Rgba {
            r: 12,
            g: 34,
            b: 56,
            a: 78,
        };

        assert_eq!(
            decode_startup_background(encode_startup_background(None)),
            None
        );
        assert_eq!(
            decode_startup_background(encode_startup_background(Some(color))),
            Some(Rgba {
                a: u8::MAX,
                ..color
            })
        );
    }

    #[test]
    fn startup_close_is_deferred_until_the_frame_is_ready() {
        let options = WindowOptions {
            center: false,
            ..WindowOptions::default()
        };
        let frame = super::super::create::FrameWindow::new(&options, None).unwrap();

        assert!(!startup_is_ready(frame.hwnd));
        defer_startup_close(frame.hwnd);
        assert!(mark_startup_ready(frame.hwnd));
        assert!(startup_is_ready(frame.hwnd));
    }
}
