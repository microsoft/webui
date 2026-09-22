// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! The native window procedure, message pump, and custom-frame geometry.

use crate::{DesktopEvent, EventResponse};
use anyhow::Result;
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller;
use windows::core::Error as WindowsError;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi;
use windows::Win32::UI::HiDpi;
use windows::Win32::UI::WindowsAndMessaging::{self, MSG};

use super::command::execute_window_command;
use super::event::{logical_dimension, physical_to_logical, size_event_transition};
use super::nonclient::{non_client_calc_size, non_client_hit_test, redraw_frame};
use super::state::{save_window_state, set_window_state, with_window_state, FrameState};
use super::webview::mirror_event;
#[cfg(feature = "application-ipc")]
use super::IPC_WAKE_MESSAGE;
use super::{WAKE_MESSAGE, WINDOW_ID};

/// Pump native messages until the window closes.
pub(super) fn message_loop() -> Result<()> {
    let mut msg = MSG::default();
    loop {
        // SAFETY: `msg` is valid writable storage for the retrieved message.
        let result = unsafe { WindowsAndMessaging::GetMessageW(&mut msg, None, 0, 0).0 };
        match result {
            -1 => return Err(WindowsError::from_thread().into()),
            0 => return Ok(()),
            // SAFETY: `msg` was filled by a successful `GetMessageW` call.
            _ => unsafe {
                let _ = WindowsAndMessaging::TranslateMessage(&msg);
                WindowsAndMessaging::DispatchMessageW(&msg);
            },
        }
    }
}

/// Native window procedure for the WebUI desktop window.
pub(super) extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    match msg {
        super::APP_WAKE_MESSAGE => {
            let tasks = super::state::with_window_state_result(hwnd, |state| {
                state.application_tasks.clone()
            });
            if let Some(tasks) = tasks {
                tasks.drain(w_param.0);
            }
            LRESULT(0)
        }
        #[cfg(feature = "application-ipc")]
        IPC_WAKE_MESSAGE => {
            let ipc = super::state::with_window_state_result(hwnd, |state| state.ipc.clone());
            // Release the native state borrow before polling COM completions.
            if let Some(ipc) = ipc {
                ipc.drain(w_param.0);
            }
            LRESULT(0)
        }
        #[cfg(feature = "application-ipc")]
        WindowsAndMessaging::WM_TIMER => {
            let ipc = super::state::with_window_state_result(hwnd, |state| state.ipc.clone());
            if let Some(ipc) = ipc {
                ipc.expire_hello(w_param.0);
            }
            LRESULT(0)
        }
        WAKE_MESSAGE => {
            drain_commands(hwnd);
            LRESULT(0)
        }
        WindowsAndMessaging::WM_NCCALCSIZE => non_client_calc_size(hwnd, msg, w_param, l_param),
        WindowsAndMessaging::WM_NCHITTEST => non_client_hit_test(hwnd, msg, w_param, l_param),
        WindowsAndMessaging::WM_DWMCOMPOSITIONCHANGED => {
            redraw_frame(hwnd);
            LRESULT(0)
        }
        WindowsAndMessaging::WM_DPICHANGED => {
            apply_suggested_rect(hwnd, l_param);
            dispatch_scale_changed(hwnd, w_param);
            LRESULT(0)
        }
        WindowsAndMessaging::WM_ACTIVATE => {
            dispatch_activation(hwnd, w_param);
            LRESULT(0)
        }
        WindowsAndMessaging::WM_SETTINGCHANGE => {
            dispatch_theme_changed(hwnd);
            // SAFETY: Setting changes must still reach default processing.
            unsafe { WindowsAndMessaging::DefWindowProcW(hwnd, msg, w_param, l_param) }
        }
        WindowsAndMessaging::WM_GETMINMAXINFO => {
            apply_size_limits(hwnd, l_param);
            LRESULT(0)
        }
        WindowsAndMessaging::WM_SIZE => {
            dispatch_size_changed(hwnd, w_param);
            LRESULT(0)
        }
        WindowsAndMessaging::WM_MOVE => {
            dispatch_moved(hwnd);
            LRESULT(0)
        }
        WindowsAndMessaging::WM_CLOSE => {
            close_window(hwnd);
            LRESULT(0)
        }
        WindowsAndMessaging::WM_ERASEBKGND => erase_background(hwnd, w_param),
        WindowsAndMessaging::WM_DESTROY => {
            destroy_window(hwnd);
            LRESULT(0)
        }
        // SAFETY: Unhandled messages are delegated to the system procedure.
        _ => unsafe { WindowsAndMessaging::DefWindowProcW(hwnd, msg, w_param, l_param) },
    }
}

/// Run every command queued from other threads.
fn drain_commands(hwnd: HWND) {
    let commands =
        super::state::with_window_state_result(hwnd, |state| state.window_handle.drain_commands())
            .unwrap_or_default();
    for command in commands {
        execute_window_command(hwnd, command);
    }
}

/// Adopt the rectangle Windows suggests for a new DPI.
fn apply_suggested_rect(hwnd: HWND, l_param: LPARAM) {
    if l_param.0 == 0 {
        return;
    }
    // SAFETY: `WM_DPICHANGED` supplies a valid `RECT` for this message.
    let rect = unsafe { *(l_param.0 as *const RECT) };
    // SAFETY: `hwnd` is a live window and the call only repositions it.
    unsafe {
        let _ = WindowsAndMessaging::SetWindowPos(
            hwnd,
            None,
            rect.left,
            rect.top,
            rect.right.saturating_sub(rect.left),
            rect.bottom.saturating_sub(rect.top),
            WindowsAndMessaging::SWP_NOZORDER | WindowsAndMessaging::SWP_NOACTIVATE,
        );
    }
}

/// Publish a scale-factor change derived from the new DPI.
fn dispatch_scale_changed(hwnd: HWND, w_param: WPARAM) {
    let dpi = u16::try_from((w_param.0 >> 16) & 0xffff).unwrap_or(96);
    let event = DesktopEvent::ScaleFactorChanged {
        scale: f64::from(dpi) / 96.0,
    };
    publish(hwnd, &event);
}

/// Publish focus gain or loss.
fn dispatch_activation(hwnd: HWND, w_param: WPARAM) {
    let event = if w_param.0 & 0xffff == 0 {
        DesktopEvent::WindowBlurred {
            window_id: WINDOW_ID,
        }
    } else {
        DesktopEvent::WindowFocused {
            window_id: WINDOW_ID,
        }
    };
    publish(hwnd, &event);
}

/// Publish a theme change after a system settings broadcast.
fn dispatch_theme_changed(hwnd: HWND) {
    let event = DesktopEvent::ThemeChanged {
        dark: system_dark(),
    };
    publish(hwnd, &event);
}

/// Report whether apps should currently use a dark theme.
pub(super) fn system_dark() -> bool {
    // SAFETY: `GetSysColor` reads a process-wide system color value.
    let color = unsafe { Gdi::GetSysColor(Gdi::COLOR_WINDOW) };
    let red = u32::from(u8::try_from(color & 0xff).unwrap_or(0));
    let green = u32::from(u8::try_from((color >> 8) & 0xff).unwrap_or(0));
    let blue = u32::from(u8::try_from((color >> 16) & 0xff).unwrap_or(0));
    (red * 299 + green * 587 + blue * 114) / 1000 < 128
}

/// Constrain interactive resizing to the manifest's size limits.
fn apply_size_limits(hwnd: HWND, l_param: LPARAM) {
    if l_param.0 == 0 {
        return;
    }
    with_window_state(hwnd, |state| {
        // SAFETY: `WM_GETMINMAXINFO` supplies a valid, writable `MINMAXINFO`
        // that stays alive for the duration of this message.
        let info = unsafe { &mut *(l_param.0 as *mut WindowsAndMessaging::MINMAXINFO) };
        if let Some(value) = state.options.min_width.and_then(|v| i32::try_from(v).ok()) {
            info.ptMinTrackSize.x = value;
        }
        if let Some(value) = state.options.min_height.and_then(|v| i32::try_from(v).ok()) {
            info.ptMinTrackSize.y = value;
        }
        if let Some(value) = state.options.max_width.and_then(|v| i32::try_from(v).ok()) {
            info.ptMaxTrackSize.x = value;
        }
        if let Some(value) = state.options.max_height.and_then(|v| i32::try_from(v).ok()) {
            info.ptMaxTrackSize.y = value;
        }
    });
}

/// Resize the WebView2 surface and publish the matching lifecycle event.
fn dispatch_size_changed(hwnd: HWND, w_param: WPARAM) {
    with_window_state(hwnd, |state| {
        let _ = set_controller_bounds(&state.controller, hwnd);
        let size = get_window_size(hwnd);
        let size_code = u32::try_from(w_param.0).unwrap_or(WindowsAndMessaging::SIZE_RESTORED);
        let (current, transition) = size_event_transition(state.window_state.get(), size_code);
        state.window_state.set(current);
        if let Some(event) = transition {
            emit(state, &event);
        }
        let dpi = window_dpi(hwnd);
        let event = DesktopEvent::WindowResized {
            window_id: WINDOW_ID,
            width: logical_dimension(size.cx, dpi),
            height: logical_dimension(size.cy, dpi),
        };
        emit(state, &event);
        save_window_state(hwnd, state);
    });
}

/// Publish a move event and persist the new position.
fn dispatch_moved(hwnd: HWND) {
    with_window_state(hwnd, |state| {
        let mut rect = RECT::default();
        // SAFETY: `hwnd` is a live window and `rect` is writable storage.
        if unsafe { WindowsAndMessaging::GetWindowRect(hwnd, &mut rect) }.is_err() {
            return;
        }
        let dpi = window_dpi(hwnd);
        emit(
            state,
            &DesktopEvent::WindowMoved {
                window_id: WINDOW_ID,
                x: physical_to_logical(rect.left, dpi),
                y: physical_to_logical(rect.top, dpi),
            },
        );
        save_window_state(hwnd, state);
    });
}

/// Offer the close request to handlers before destroying the window.
fn close_window(hwnd: HWND) {
    let prevented = super::state::with_window_state_result(hwnd, |state| {
        let event = DesktopEvent::WindowCloseRequested {
            window_id: WINDOW_ID,
        };
        let prevented = state.events.dispatch(&event) == EventResponse::PreventDefault;
        mirror_event(&state.webview, &event);
        prevented
    })
    .unwrap_or(false);
    if prevented {
        return;
    }
    // SAFETY: `hwnd` is a live window and destruction is the normal close path.
    unsafe {
        let _ = WindowsAndMessaging::DestroyWindow(hwnd);
    }
}

/// Paint the configured background so there is no flash before first paint.
fn erase_background(hwnd: HWND, w_param: WPARAM) -> LRESULT {
    let Some(color) =
        super::state::with_window_state_result(hwnd, |state| state.options.background).flatten()
    else {
        return LRESULT(0);
    };
    let rgb = u32::from(color.r) | (u32::from(color.g) << 8) | (u32::from(color.b) << 16);
    let hdc = Gdi::HDC(w_param.0 as *mut std::ffi::c_void);
    let mut rect = RECT::default();
    // SAFETY: `hwnd` is a live window and `rect` is writable storage.
    if unsafe { WindowsAndMessaging::GetClientRect(hwnd, &mut rect) }.is_err() {
        return LRESULT(0);
    }
    // SAFETY: Windows supplies the device context in `w_param` for this
    // message, `rect` is initialized, and the brush is deleted before return.
    unsafe {
        let brush = Gdi::CreateSolidBrush(windows::Win32::Foundation::COLORREF(rgb));
        let _ = Gdi::FillRect(hdc, &rect, brush);
        let _ = Gdi::DeleteObject(brush.into());
    }
    LRESULT(1)
}

/// Publish the final close event and release the frame state.
fn destroy_window(hwnd: HWND) {
    #[cfg(feature = "application-ipc")]
    {
        let ipc = super::state::with_window_state_result(hwnd, |state| state.ipc.clone());
        if let Some(ipc) = ipc {
            ipc.close();
        }
    }
    with_window_state(hwnd, |state| {
        emit(
            state,
            &DesktopEvent::WindowClosed {
                window_id: WINDOW_ID,
            },
        );
    });
    drop(set_window_state(hwnd, None));
    // SAFETY: Posting quit from WM_DESTROY ends this thread's message loop.
    unsafe { WindowsAndMessaging::PostQuitMessage(0) };
}

/// Dispatch an event to handlers and mirror it into web content.
pub(super) fn emit(state: &FrameState, event: &DesktopEvent) {
    let _ = state.events.dispatch(event);
    mirror_event(&state.webview, event);
}

/// Dispatch an event when the frame state is installed.
pub(super) fn publish(hwnd: HWND, event: &DesktopEvent) {
    with_window_state(hwnd, |state| emit(state, event));
}

/// Match the WebView2 surface to the window's client area.
pub(super) fn set_controller_bounds(
    controller: &ICoreWebView2Controller,
    hwnd: HWND,
) -> Result<()> {
    let size = get_window_size(hwnd);
    // SAFETY: `controller` is a live WebView2 controller for this window.
    unsafe {
        controller.SetBounds(RECT {
            left: 0,
            top: 0,
            right: size.cx,
            bottom: size.cy,
        })?;
    }
    Ok(())
}

/// Return the window's current DPI for physical-to-logical conversion.
///
/// `GetDpiForWindow` returns 0 for an invalid window; callers treat 0 as the
/// 96-DPI baseline rather than dividing by zero.
fn window_dpi(hwnd: HWND) -> u32 {
    // SAFETY: `hwnd` is a live window and `GetDpiForWindow` only reads its
    // DPI awareness context.
    unsafe { HiDpi::GetDpiForWindow(hwnd) }
}

/// Return the window's client size in physical pixels.
pub(super) fn get_window_size(hwnd: HWND) -> SIZE {
    let mut client_rect = RECT::default();
    // SAFETY: `hwnd` is a live window and `client_rect` is writable storage.
    let _ = unsafe { WindowsAndMessaging::GetClientRect(hwnd, &mut client_rect) };
    SIZE {
        cx: client_rect.right.saturating_sub(client_rect.left),
        cy: client_rect.bottom.saturating_sub(client_rect.top),
    }
}
