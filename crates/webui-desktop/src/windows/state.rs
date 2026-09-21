// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! UI-thread window state: the per-window frame record, `GWLP_USERDATA`
//! storage, display enumeration, and persisted geometry.

use std::cell::Cell;

use crate::{
    DisplayBounds, EventRegistry, TitlebarStyle, WindowHandle, WindowOptions, WindowState,
    WindowStateStore,
};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2, ICoreWebView2Controller, ICoreWebView2NavigationCompletedEventHandler,
    ICoreWebView2NavigationStartingEventHandler, ICoreWebView2WebMessageReceivedEventHandler,
    ICoreWebView2WebResourceRequestedEventHandler,
};
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi;
use windows::Win32::UI::WindowsAndMessaging::{
    self, WINDOW_EX_STYLE, WINDOW_LONG_PTR_INDEX, WINDOW_STYLE,
};

/// The native state reported by `WM_SIZE`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WindowSizeState {
    /// The window has its ordinary restored frame.
    Normal,
    /// The window fills its work area.
    Maximized,
    /// The window is minimized to the taskbar.
    Minimized,
}

/// Window geometry and styles saved before entering fullscreen.
#[derive(Clone, Copy)]
pub(super) struct SavedFrame {
    /// Window style captured before the fullscreen transition.
    pub(super) style: WINDOW_STYLE,
    /// Extended window style captured before the fullscreen transition.
    pub(super) ex_style: WINDOW_EX_STYLE,
    /// Screen rectangle captured before the fullscreen transition.
    pub(super) rect: RECT,
    /// Whether the window was maximized before the fullscreen transition.
    pub(super) maximized: bool,
}

/// Per-window state owned by the native window procedure.
pub(super) struct FrameState {
    /// WebView2 controller that hosts the app content.
    pub(super) controller: ICoreWebView2Controller,
    /// Retained navigation policy handler.
    pub(super) _navigation_starting: ICoreWebView2NavigationStartingEventHandler,
    /// Retained navigation completion handler.
    pub(super) _navigation_completed: ICoreWebView2NavigationCompletedEventHandler,
    /// Retained script-message handler.
    pub(super) _web_message_received: ICoreWebView2WebMessageReceivedEventHandler,
    /// Retained resource interception handler.
    pub(super) _web_resource_requested: ICoreWebView2WebResourceRequestedEventHandler,
    /// Lifecycle event registry shared with app code.
    pub(super) events: EventRegistry,
    /// Sendable command queue drained on the UI thread.
    pub(super) window_handle: WindowHandle,
    /// Window configuration from the desktop manifest.
    pub(super) options: WindowOptions,
    /// WebView2 instance used to mirror events into web content.
    pub(super) webview: ICoreWebView2,
    /// Geometry store, present only when `remember_state` is enabled.
    pub(super) store: Option<WindowStateStore>,
    /// Saved frame captured while the window is fullscreen.
    pub(super) fullscreen: Cell<Option<SavedFrame>>,
    /// Most recent state reported by `WM_SIZE`.
    pub(super) window_state: Cell<WindowSizeState>,
}

impl FrameState {
    /// Return the requested titlebar style.
    pub(super) fn titlebar(&self) -> &TitlebarStyle {
        &self.options.titlebar
    }
}

/// Read the current native size state while constructing a frame record.
pub(super) fn initial_window_size_state(hwnd: HWND) -> WindowSizeState {
    // SAFETY: `hwnd` is a live window; `IsIconic` and `IsZoomed` only read its state.
    if unsafe { WindowsAndMessaging::IsIconic(hwnd) }.as_bool() {
        WindowSizeState::Minimized
    // SAFETY: `hwnd` is a live window; `IsZoomed` only reads its state.
    } else if unsafe { WindowsAndMessaging::IsZoomed(hwnd) }.as_bool() {
        WindowSizeState::Maximized
    } else {
        WindowSizeState::Normal
    }
}

/// Return every display rectangle, used to reject stale saved geometry.
pub(super) fn display_bounds() -> Vec<DisplayBounds> {
    let mut displays: Vec<DisplayBounds> = Vec::with_capacity(4);
    let pointer = LPARAM(std::ptr::from_mut(&mut displays) as isize);
    // SAFETY: `enumerate_display` only dereferences the `Vec<DisplayBounds>`
    // pointer passed here, which stays alive and exclusively borrowed for the
    // duration of this synchronous enumeration.
    let _ = unsafe { Gdi::EnumDisplayMonitors(None, None, Some(enumerate_display), pointer) };
    displays
}

/// Collect one monitor rectangle into the caller's vector.
///
/// # Safety
///
/// Called only by `EnumDisplayMonitors` from [`display_bounds`], which passes a
/// valid monitor rectangle and a `data` value derived from a live, exclusively
/// borrowed `Vec<DisplayBounds>`.
unsafe extern "system" fn enumerate_display(
    _monitor: Gdi::HMONITOR,
    _hdc: Gdi::HDC,
    rect: *mut RECT,
    data: LPARAM,
) -> windows::core::BOOL {
    if rect.is_null() || data.0 == 0 {
        return true.into();
    }
    // SAFETY: Windows supplies a valid monitor rectangle, and `data` carries the
    // vector pointer created in `display_bounds` for this enumeration only.
    unsafe {
        let rect = *rect;
        let displays = &mut *(data.0 as *mut Vec<DisplayBounds>);
        let width = rect.right.saturating_sub(rect.left);
        let height = rect.bottom.saturating_sub(rect.top);
        if let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) {
            displays.push(DisplayBounds {
                x: rect.left,
                y: rect.top,
                width,
                height,
            });
        }
    }
    true.into()
}

/// Load previously saved geometry when the window opts into persistence.
///
/// Returns `None` when persistence is disabled, no state exists, or the saved
/// rectangle no longer intersects a connected display.
pub(super) fn load_saved_state(store: Option<&WindowStateStore>) -> Option<WindowState> {
    let store = store?;
    match store.load_valid(&display_bounds()) {
        Ok(state) => state,
        Err(error) => {
            eprintln!("WebUI: failed to restore window state: {error}");
            None
        }
    }
}

/// Persist the current window rectangle when persistence is enabled.
pub(super) fn save_window_state(hwnd: HWND, state: &FrameState) {
    let Some(store) = state.store.as_ref() else {
        return;
    };
    if state.fullscreen.get().is_some() {
        return;
    }
    // SAFETY: `hwnd` is a live window; `IsIconic` only reads window state.
    if unsafe { WindowsAndMessaging::IsIconic(hwnd) }.as_bool() {
        // A minimized window reports an off-screen rectangle that would fail
        // display validation on the next launch.
        return;
    }
    let mut rect = RECT::default();
    // SAFETY: `hwnd` is the live window owning this state and `rect` is valid
    // writable stack storage for the returned rectangle.
    if unsafe { WindowsAndMessaging::GetWindowRect(hwnd, &mut rect) }.is_err() {
        return;
    }
    // SAFETY: `hwnd` is a live window; `IsZoomed` only reads window state.
    let maximized = unsafe { WindowsAndMessaging::IsZoomed(hwnd) }.as_bool();
    let (Ok(width), Ok(height)) = (
        u32::try_from(rect.right.saturating_sub(rect.left)),
        u32::try_from(rect.bottom.saturating_sub(rect.top)),
    ) else {
        return;
    };
    let _ = store.save(&WindowState {
        x: rect.left,
        y: rect.top,
        width,
        height,
        maximized,
    });
}

/// Install the frame state pointer, returning any previously installed state.
pub(super) fn set_window_state(
    hwnd: HWND,
    state: Option<Box<FrameState>>,
) -> Option<Box<FrameState>> {
    let value = state.map_or(0_isize, |state| Box::into_raw(state) as isize);
    // SAFETY: `hwnd` is a live window owned by this process and `GWLP_USERDATA`
    // is reserved for this backend's frame pointer.
    let previous = unsafe { set_window_long(hwnd, WindowsAndMessaging::GWLP_USERDATA, value) };
    if previous == 0 {
        None
    } else {
        // SAFETY: A non-zero previous value was produced by `Box::into_raw` in
        // an earlier call, so reclaiming it as a `Box` restores sole ownership.
        Some(unsafe { Box::from_raw(previous as *mut FrameState) })
    }
}

/// Run a closure against the installed frame state and return its value.
pub(super) fn with_window_state_result<T>(
    hwnd: HWND,
    f: impl FnOnce(&FrameState) -> T,
) -> Option<T> {
    // SAFETY: Reading `GWLP_USERDATA` on a live window returns the pointer this
    // backend installed, or zero before installation and after WM_DESTROY.
    let data = unsafe { get_window_long(hwnd, WindowsAndMessaging::GWLP_USERDATA) };
    if data == 0 {
        return None;
    }
    // SAFETY: `data` came from `Box::into_raw` in `set_window_state`, the box is
    // cleared only in WM_DESTROY, and the UI thread never aliases it mutably.
    Some(f(unsafe { &*(data as *const FrameState) }))
}

/// Run a closure against the installed frame state.
pub(super) fn with_window_state(hwnd: HWND, f: impl FnOnce(&FrameState)) {
    let _ = with_window_state_result(hwnd, f);
}

/// Read the window style bits of a live window.
pub(super) fn window_style_bits(hwnd: HWND, index: WINDOW_LONG_PTR_INDEX) -> u32 {
    // SAFETY: `hwnd` is a live window and style indices only read window data.
    let bits = unsafe { WindowsAndMessaging::GetWindowLongW(hwnd, index) };
    bits.cast_unsigned()
}

#[allow(non_snake_case)]
#[cfg(target_pointer_width = "32")]
/// Store a pointer-sized value in window memory.
///
/// # Safety
///
/// `hwnd` must be a live window owned by this process and `index` must address
/// storage reserved for this backend.
unsafe fn set_window_long(hwnd: HWND, index: WINDOW_LONG_PTR_INDEX, value: isize) -> isize {
    // SAFETY: Guaranteed by this function's contract.
    unsafe { WindowsAndMessaging::SetWindowLongW(hwnd, index, value as _) as _ }
}

#[allow(non_snake_case)]
#[cfg(target_pointer_width = "64")]
/// Store a pointer-sized value in window memory.
///
/// # Safety
///
/// `hwnd` must be a live window owned by this process and `index` must address
/// storage reserved for this backend.
unsafe fn set_window_long(hwnd: HWND, index: WINDOW_LONG_PTR_INDEX, value: isize) -> isize {
    // SAFETY: Guaranteed by this function's contract.
    unsafe { WindowsAndMessaging::SetWindowLongPtrW(hwnd, index, value) }
}

#[allow(non_snake_case)]
#[cfg(target_pointer_width = "32")]
/// Read a pointer-sized value from window memory.
///
/// # Safety
///
/// `hwnd` must be a live window owned by this process and `index` must address
/// storage reserved for this backend.
unsafe fn get_window_long(hwnd: HWND, index: WINDOW_LONG_PTR_INDEX) -> isize {
    // SAFETY: Guaranteed by this function's contract.
    unsafe { WindowsAndMessaging::GetWindowLongW(hwnd, index) as _ }
}

#[allow(non_snake_case)]
#[cfg(target_pointer_width = "64")]
/// Read a pointer-sized value from window memory.
///
/// # Safety
///
/// `hwnd` must be a live window owned by this process and `index` must address
/// storage reserved for this backend.
unsafe fn get_window_long(hwnd: HWND, index: WINDOW_LONG_PTR_INDEX) -> isize {
    // SAFETY: Guaranteed by this function's contract.
    unsafe { WindowsAndMessaging::GetWindowLongPtrW(hwnd, index) }
}
