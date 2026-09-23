// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Native window creation, style selection, and initial geometry.

use crate::{TitlebarStyle, WindowOptions, WindowState};
use anyhow::{Context, Result};
use webview2_com::CoTaskMemPWSTR;
use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND};
use windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
use windows::Win32::System::LibraryLoader;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::WindowsAndMessaging::{
    self, WNDCLASSW, WS_CAPTION, WS_MAXIMIZEBOX, WS_OVERLAPPEDWINDOW, WS_THICKFRAME,
};

use super::command::center_window;
use super::message::window_proc;

/// Owner of the native top-level window handle.
pub(super) struct FrameWindow {
    /// Handle to the created top-level window.
    pub(super) hwnd: HWND,
}

impl FrameWindow {
    /// Create the native window, applying saved geometry when available.
    pub(super) fn new(window: &WindowOptions, saved: Option<&WindowState>) -> Result<Self> {
        let title = CoTaskMemPWSTR::from(window.title.as_str());
        let (x, y, width, height) = initial_geometry(window, saved)?;
        // SAFETY: The class name, window title, and module handle are valid for
        // this call, and `window_proc` has the required `WNDPROC` signature.
        let hwnd = unsafe {
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                lpszClassName: w!("WebUIDesktopWindow"),
                hCursor: WindowsAndMessaging::LoadCursorW(None, WindowsAndMessaging::IDC_ARROW)
                    .unwrap_or_default(),
                ..Default::default()
            };
            WindowsAndMessaging::RegisterClassW(&class);
            WindowsAndMessaging::CreateWindowExW(
                Default::default(),
                w!("WebUIDesktopWindow"),
                *title.as_ref().as_pcwstr(),
                window_style(window),
                x,
                y,
                width,
                height,
                None,
                None,
                LibraryLoader::GetModuleHandleW(None)
                    .ok()
                    .map(|handle| HINSTANCE(handle.0)),
                None,
            )?
        };
        let frame = Self { hwnd };
        frame.apply_custom_frame(window);
        frame.apply_initial_state(window, saved);
        Ok(frame)
    }

    /// Ask DWM to draw the system shadow and caption buttons over the client
    /// area for window styles that keep native controls.
    fn apply_custom_frame(&self, window: &WindowOptions) {
        if matches!(window.titlebar, TitlebarStyle::Native | TitlebarStyle::None) {
            return;
        }
        let margins = MARGINS {
            cxLeftWidth: 0,
            cxRightWidth: 0,
            cyTopHeight: 1,
            cyBottomHeight: 0,
        };
        // SAFETY: `self.hwnd` is the live window just created and `margins` is
        // an initialized value read only for the duration of this call.
        let _ = unsafe { DwmExtendFrameIntoClientArea(self.hwnd, &margins) };
        // SAFETY: The window is live; the reposition only forces the frame to
        // be recalculated so `WM_NCCALCSIZE` runs with the new styles.
        unsafe {
            let _ = WindowsAndMessaging::SetWindowPos(
                self.hwnd,
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

    /// Apply centering, always-on-top, and the maximized start state.
    fn apply_initial_state(&self, window: &WindowOptions, saved: Option<&WindowState>) {
        if saved.is_none() && window.center {
            center_window(self.hwnd);
        }
        if window.always_on_top {
            // SAFETY: The window is live and only its z-order changes.
            unsafe {
                let _ = WindowsAndMessaging::SetWindowPos(
                    self.hwnd,
                    Some(WindowsAndMessaging::HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    WindowsAndMessaging::SWP_NOMOVE | WindowsAndMessaging::SWP_NOSIZE,
                );
            }
        }
        if window.maximized || saved.is_some_and(|state| state.maximized) {
            // SAFETY: The window is live and the command only changes its state.
            unsafe {
                let _ =
                    WindowsAndMessaging::ShowWindow(self.hwnd, WindowsAndMessaging::SW_MAXIMIZE);
            }
        }
    }
}

/// Resolve the creation rectangle from saved state or manifest options.
fn initial_geometry(
    window: &WindowOptions,
    saved: Option<&WindowState>,
) -> Result<(i32, i32, i32, i32)> {
    if let Some(saved) = saved {
        let width = i32::try_from(saved.width).context("saved window width exceeds Win32 range")?;
        let height =
            i32::try_from(saved.height).context("saved window height exceeds Win32 range")?;
        return Ok((saved.x, saved.y, width, height));
    }
    let width = i32::try_from(window.width).context("window width exceeds Win32 range")?;
    let height = i32::try_from(window.height).context("window height exceeds Win32 range")?;
    Ok((
        WindowsAndMessaging::CW_USEDEFAULT,
        WindowsAndMessaging::CW_USEDEFAULT,
        width,
        height,
    ))
}

/// Select the window style bits implied by the manifest options.
pub(super) fn window_style(window: &WindowOptions) -> WindowsAndMessaging::WINDOW_STYLE {
    let mut style = WS_OVERLAPPEDWINDOW;
    if !window.resizable {
        style &= !WS_THICKFRAME;
        style &= !WS_MAXIMIZEBOX;
    }
    if matches!(window.titlebar, TitlebarStyle::None) {
        // Keep WS_THICKFRAME for resizable frameless windows: the caption is
        // removed but the resize frame drives WM_NCHITTEST borders.
        style &= !WS_CAPTION;
    }
    style
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn frameless_style_keeps_resize_frame_without_caption() {
        let style = window_style(&WindowOptions {
            titlebar: TitlebarStyle::None,
            ..WindowOptions::default()
        });

        assert_eq!(style & WS_CAPTION, WindowsAndMessaging::WINDOW_STYLE(0));
        assert_eq!(style & WS_THICKFRAME, WS_THICKFRAME);
    }

    #[test]
    fn non_resizable_style_drops_resize_affordances() {
        let style = window_style(&WindowOptions {
            resizable: false,
            ..WindowOptions::default()
        });

        assert_eq!(style & WS_THICKFRAME, WindowsAndMessaging::WINDOW_STYLE(0));
        assert_eq!(style & WS_MAXIMIZEBOX, WindowsAndMessaging::WINDOW_STYLE(0));
        assert_eq!(style & WS_CAPTION, WS_CAPTION);
    }

    #[test]
    fn saved_geometry_overrides_manifest_size() {
        let saved = WindowState {
            x: 30,
            y: 40,
            width: 900,
            height: 700,
            maximized: false,
        };

        let geometry = initial_geometry(&WindowOptions::default(), Some(&saved)).unwrap();

        assert_eq!(geometry, (30, 40, 900, 700));
    }

    #[test]
    fn default_geometry_uses_system_placement() {
        let geometry = initial_geometry(&WindowOptions::default(), None).unwrap();

        assert_eq!(
            geometry,
            (
                WindowsAndMessaging::CW_USEDEFAULT,
                WindowsAndMessaging::CW_USEDEFAULT,
                1200,
                800
            )
        );
    }
}
