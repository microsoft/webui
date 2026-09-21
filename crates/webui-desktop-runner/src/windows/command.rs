// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! UI-thread execution of queued [`WindowCommand`]s and host script messages.

use webui_desktop::{DesktopEvent, DesktopHostMessage, WindowCommand};
use webview2_com::CoTaskMemPWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi;
use windows::Win32::UI::Input::KeyboardAndMouse;
use windows::Win32::UI::WindowsAndMessaging::{self, WINDOW_EX_STYLE, WINDOW_STYLE};

use super::state::{with_window_state, FrameState, SavedFrame};
use super::{message, WINDOW_ID};

/// Execute one queued command on the UI thread.
pub(super) fn execute_window_command(hwnd: HWND, command: WindowCommand) {
    match command {
        WindowCommand::SetTitle(title) => set_title(hwnd, &title),
        WindowCommand::SetSize { width, height } => set_size(hwnd, width, height),
        WindowCommand::Minimize => show_window(hwnd, WindowsAndMessaging::SW_MINIMIZE),
        WindowCommand::Maximize => show_window(hwnd, WindowsAndMessaging::SW_MAXIMIZE),
        WindowCommand::Unmaximize => show_window(hwnd, WindowsAndMessaging::SW_RESTORE),
        WindowCommand::Close => post_close(hwnd),
        WindowCommand::Focus => focus_window(hwnd),
        WindowCommand::Center => center_window(hwnd),
        WindowCommand::StartDrag => start_drag(hwnd),
        WindowCommand::SetAlwaysOnTop(value) => set_always_on_top(hwnd, value),
        WindowCommand::SetFullscreen(value) => {
            with_window_state(hwnd, |state| set_fullscreen(hwnd, state, value));
        }
    }
}

/// Execute a message posted by injected web content.
pub(super) fn execute_host_message(hwnd: HWND, message: DesktopHostMessage) {
    match message {
        DesktopHostMessage::StartDrag => start_drag(hwnd),
        DesktopHostMessage::Minimize => show_window(hwnd, WindowsAndMessaging::SW_MINIMIZE),
        DesktopHostMessage::ToggleMaximize => toggle_maximize(hwnd),
        DesktopHostMessage::Close => post_close(hwnd),
    }
}

/// Restore a maximized window or maximize a restored one.
pub(super) fn toggle_maximize(hwnd: HWND) {
    // SAFETY: `hwnd` is a live window and `IsZoomed` only reads window state.
    let zoomed = unsafe { WindowsAndMessaging::IsZoomed(hwnd) }.as_bool();
    let command = if zoomed {
        WindowsAndMessaging::SC_RESTORE
    } else {
        WindowsAndMessaging::SC_MAXIMIZE
    };
    let Ok(command) = usize::try_from(command) else {
        return;
    };
    // SAFETY: `hwnd` is a live window and `WM_SYSCOMMAND` with `SC_RESTORE` or
    // `SC_MAXIMIZE` is the documented way to drive the system restore/maximize
    // transition, including its animation and caption-button state.
    unsafe {
        WindowsAndMessaging::SendMessageW(
            hwnd,
            WindowsAndMessaging::WM_SYSCOMMAND,
            Some(WPARAM(command)),
            Some(LPARAM(0)),
        );
    }
}

/// Enter or leave fullscreen, saving and restoring the previous frame.
pub(super) fn set_fullscreen(hwnd: HWND, state: &FrameState, enable: bool) {
    if enable {
        if enter_fullscreen(hwnd, state) {
            message::emit(
                state,
                &DesktopEvent::WindowEnteredFullscreen {
                    window_id: WINDOW_ID,
                },
            );
        }
    } else if leave_fullscreen(hwnd, state) {
        message::emit(
            state,
            &DesktopEvent::WindowLeftFullscreen {
                window_id: WINDOW_ID,
            },
        );
    }
}

/// Save the current frame, strip the border styles, and fill the monitor.
fn enter_fullscreen(hwnd: HWND, state: &FrameState) -> bool {
    if state.fullscreen.get().is_some() {
        return false;
    }
    let style = WINDOW_STYLE(super::state::window_style_bits(
        hwnd,
        WindowsAndMessaging::GWL_STYLE,
    ));
    let ex_style = WINDOW_EX_STYLE(super::state::window_style_bits(
        hwnd,
        WindowsAndMessaging::GWL_EXSTYLE,
    ));
    let mut rect = RECT::default();
    // SAFETY: `hwnd` is a live window and `rect` is valid writable storage.
    if unsafe { WindowsAndMessaging::GetWindowRect(hwnd, &mut rect) }.is_err() {
        return false;
    }
    // SAFETY: `hwnd` is a live window and `IsZoomed` only reads window state.
    let maximized = unsafe { WindowsAndMessaging::IsZoomed(hwnd) }.as_bool();
    let Some(monitor) = monitor_rect(hwnd, false) else {
        return false;
    };
    state.fullscreen.set(Some(SavedFrame {
        style,
        ex_style,
        rect,
        maximized,
    }));
    let fullscreen_style = style
        & !WindowsAndMessaging::WS_OVERLAPPEDWINDOW
        & !WindowsAndMessaging::WS_CAPTION
        & !WindowsAndMessaging::WS_THICKFRAME;
    set_style(hwnd, fullscreen_style);
    apply_rect(hwnd, monitor);
    true
}

/// Restore the styles and rectangle captured before fullscreen.
fn leave_fullscreen(hwnd: HWND, state: &FrameState) -> bool {
    let Some(saved) = state.fullscreen.take() else {
        return false;
    };
    set_style(hwnd, saved.style);
    set_ex_style(hwnd, saved.ex_style);
    if saved.maximized {
        show_window(hwnd, WindowsAndMessaging::SW_MAXIMIZE);
    } else {
        apply_rect(hwnd, saved.rect);
    }
    true
}

/// Replace the window style and request a non-client frame recalculation.
fn set_style(hwnd: HWND, style: WINDOW_STYLE) {
    // SAFETY: `hwnd` is a live window and `GWL_STYLE` stores the style bits.
    unsafe {
        WindowsAndMessaging::SetWindowLongW(
            hwnd,
            WindowsAndMessaging::GWL_STYLE,
            style.0.cast_signed(),
        );
    }
}

/// Replace the extended window style.
fn set_ex_style(hwnd: HWND, style: WINDOW_EX_STYLE) {
    // SAFETY: `hwnd` is a live window and `GWL_EXSTYLE` stores the extended bits.
    unsafe {
        WindowsAndMessaging::SetWindowLongW(
            hwnd,
            WindowsAndMessaging::GWL_EXSTYLE,
            style.0.cast_signed(),
        );
    }
}

/// Move and size the window to an absolute screen rectangle.
fn apply_rect(hwnd: HWND, rect: RECT) {
    // SAFETY: `hwnd` is a live window; the call only repositions it and forces
    // the frame to be recomputed after a style change.
    unsafe {
        let _ = WindowsAndMessaging::SetWindowPos(
            hwnd,
            None,
            rect.left,
            rect.top,
            rect.right.saturating_sub(rect.left),
            rect.bottom.saturating_sub(rect.top),
            WindowsAndMessaging::SWP_NOZORDER
                | WindowsAndMessaging::SWP_NOACTIVATE
                | WindowsAndMessaging::SWP_FRAMECHANGED,
        );
    }
}

/// Return the full or work-area rectangle of the window's nearest monitor.
pub(super) fn monitor_rect(hwnd: HWND, work_area: bool) -> Option<RECT> {
    // SAFETY: `hwnd` is a live window; `MonitorFromWindow` only reads placement.
    let monitor = unsafe { Gdi::MonitorFromWindow(hwnd, Gdi::MONITOR_DEFAULTTONEAREST) };
    let mut info = Gdi::MONITORINFO {
        cbSize: u32::try_from(std::mem::size_of::<Gdi::MONITORINFO>()).unwrap_or(40),
        ..Default::default()
    };
    // SAFETY: `info` is initialized with its required `cbSize` and is valid
    // writable storage for the monitor rectangles.
    let ok = unsafe { Gdi::GetMonitorInfoW(monitor, &mut info) }.as_bool();
    ok.then_some(if work_area {
        info.rcWork
    } else {
        info.rcMonitor
    })
}

/// Center the window inside its monitor's work area.
pub(super) fn center_window(hwnd: HWND) {
    let mut rect = RECT::default();
    // SAFETY: `hwnd` is a live window and `rect` is valid writable storage.
    if unsafe { WindowsAndMessaging::GetWindowRect(hwnd, &mut rect) }.is_err() {
        return;
    }
    let Some(work) = monitor_rect(hwnd, true) else {
        return;
    };
    let width = rect.right.saturating_sub(rect.left);
    let height = rect.bottom.saturating_sub(rect.top);
    let x = work.left + (work.right.saturating_sub(work.left).saturating_sub(width)) / 2;
    let y = work.top + (work.bottom.saturating_sub(work.top).saturating_sub(height)) / 2;
    // SAFETY: `hwnd` is a live window and the call only moves it.
    unsafe {
        let _ = WindowsAndMessaging::SetWindowPos(
            hwnd,
            None,
            x,
            y,
            0,
            0,
            WindowsAndMessaging::SWP_NOSIZE | WindowsAndMessaging::SWP_NOZORDER,
        );
    }
}

/// Set the native window title.
fn set_title(hwnd: HWND, title: &str) {
    let title = CoTaskMemPWSTR::from(title);
    // SAFETY: `hwnd` is a live window and the wide string stays alive for this
    // synchronous call, which copies the text into the window.
    unsafe {
        let _ = WindowsAndMessaging::SetWindowTextW(hwnd, *title.as_ref().as_pcwstr());
    }
}

/// Resize the window without moving it.
fn set_size(hwnd: HWND, width: u32, height: u32) {
    let (Ok(width), Ok(height)) = (i32::try_from(width), i32::try_from(height)) else {
        return;
    };
    // SAFETY: `hwnd` is a live window and the call only resizes it.
    unsafe {
        let _ = WindowsAndMessaging::SetWindowPos(
            hwnd,
            None,
            0,
            0,
            width,
            height,
            WindowsAndMessaging::SWP_NOMOVE | WindowsAndMessaging::SWP_NOZORDER,
        );
    }
}

/// Apply a show command to the window.
fn show_window(hwnd: HWND, command: WindowsAndMessaging::SHOW_WINDOW_CMD) {
    // SAFETY: `hwnd` is a live window and show commands only change its state.
    unsafe {
        let _ = WindowsAndMessaging::ShowWindow(hwnd, command);
    }
}

/// Ask the window to close, honoring the `WM_CLOSE` prevention path.
fn post_close(hwnd: HWND) {
    // SAFETY: `hwnd` is a live window; posting WM_CLOSE re-enters the window
    // procedure, which dispatches `WindowCloseRequested` before destroying it.
    unsafe {
        let _ = WindowsAndMessaging::PostMessageW(
            Some(hwnd),
            WindowsAndMessaging::WM_CLOSE,
            WPARAM(0),
            LPARAM(0),
        );
    }
}

/// Give the window keyboard focus.
fn focus_window(hwnd: HWND) {
    // SAFETY: `hwnd` is a live window owned by this thread.
    unsafe {
        let _ = KeyboardAndMouse::SetFocus(Some(hwnd));
    }
}

/// Begin a system caption drag from a web-content drag region.
fn start_drag(hwnd: HWND) {
    let Ok(caption) = usize::try_from(WindowsAndMessaging::HTCAPTION) else {
        return;
    };
    let mut cursor = POINT::default();
    // SAFETY: `cursor` is valid writable storage for the cursor position.
    if unsafe { WindowsAndMessaging::GetCursorPos(&mut cursor) }.is_err() {
        return;
    }
    // The system move loop anchors on the packed screen position, so the press
    // must be reported where the pointer actually is.
    let anchor = pack_point(cursor);
    // SAFETY: `hwnd` is a live window. Releasing capture and forwarding a
    // non-client caption press is the documented way to hand an in-progress
    // pointer interaction to the system move loop.
    unsafe {
        let _ = KeyboardAndMouse::ReleaseCapture();
        WindowsAndMessaging::SendMessageW(
            hwnd,
            WindowsAndMessaging::WM_NCLBUTTONDOWN,
            Some(WPARAM(caption)),
            Some(LPARAM(anchor)),
        );
    }
}

/// Pack a screen point into the `LPARAM` layout Windows expects.
///
/// Mirrors `MAKELPARAM`: the low word carries x and the high word carries y.
/// `isize` has no `From<u16>` impl, so the truncated halves are combined as a
/// `u32` and converted once.
fn pack_point(point: POINT) -> isize {
    let x = point.x.cast_unsigned() & 0xffff;
    let y = point.y.cast_unsigned() & 0xffff;
    isize::try_from((y << 16) | x).unwrap_or_default()
}

/// Raise or lower the window relative to ordinary windows.
fn set_always_on_top(hwnd: HWND, value: bool) {
    let insert_after = if value {
        WindowsAndMessaging::HWND_TOPMOST
    } else {
        WindowsAndMessaging::HWND_NOTOPMOST
    };
    // SAFETY: `hwnd` is a live window and only its z-order changes.
    unsafe {
        let _ = WindowsAndMessaging::SetWindowPos(
            hwnd,
            Some(insert_after),
            0,
            0,
            0,
            0,
            WindowsAndMessaging::SWP_NOMOVE | WindowsAndMessaging::SWP_NOSIZE,
        );
    }
}

/// Unpack the signed screen coordinates Windows packs into an `LPARAM`.
pub(super) fn screen_point(l_param: LPARAM) -> POINT {
    let bits = l_param.0;
    let low = u16::try_from(bits & 0xffff).unwrap_or_default();
    let high = u16::try_from((bits >> 16) & 0xffff).unwrap_or_default();
    POINT {
        x: i32::from(low.cast_signed()),
        y: i32::from(high.cast_signed()),
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn packed_points_round_trip_through_lparam() {
        for point in [
            POINT { x: 0, y: 0 },
            POINT { x: 1280, y: 720 },
            POINT { x: -1920, y: -40 },
        ] {
            let unpacked = screen_point(LPARAM(pack_point(point)));
            assert_eq!((unpacked.x, unpacked.y), (point.x, point.y));
        }
    }
}
