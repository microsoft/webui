// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Native window creation, style selection, and initial geometry.

use crate::{TitlebarStyle, WindowOptions, WindowState};
use anyhow::{Context, Result};
use webview2_com::CoTaskMemPWSTR;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND};
use windows::Win32::System::LibraryLoader;
use windows::Win32::UI::WindowsAndMessaging::{
    self, WNDCLASSW, WS_MAXIMIZEBOX, WS_OVERLAPPEDWINDOW, WS_THICKFRAME,
};

use super::command::center_window;
use super::message::window_proc;
use super::nonclient::CUSTOM_FRAME_BYTES;

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
        let mut style = window_style(window);
        if window.maximized || saved.is_some_and(|state| state.maximized) {
            style |= WindowsAndMessaging::WS_MAXIMIZE;
        }
        // SAFETY: The executable module remains loaded for the lifetime of the
        // window; Win32 treats this low pointer value as resource ID 1, not an
        // address to dereference. The resource is optional for custom runners.
        let instance = unsafe { HINSTANCE(LibraryLoader::GetModuleHandleW(None)?.0) };
        let icon = unsafe {
            WindowsAndMessaging::LoadIconW(
                Some(instance),
                PCWSTR::from_raw(std::ptr::without_provenance(1)),
            )
            .unwrap_or_default()
        };
        // SAFETY: The class name, window title, and module handle are valid for
        // this call, and `window_proc` has the required `WNDPROC` signature.
        let hwnd = unsafe {
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                lpszClassName: w!("WebUIDesktopWindow"),
                cbWndExtra: CUSTOM_FRAME_BYTES,
                hInstance: instance,
                hIcon: icon,
                hCursor: WindowsAndMessaging::LoadCursorW(None, WindowsAndMessaging::IDC_ARROW)
                    .unwrap_or_default(),
                ..Default::default()
            };
            WindowsAndMessaging::RegisterClassW(&class);
            WindowsAndMessaging::CreateWindowExW(
                Default::default(),
                w!("WebUIDesktopWindow"),
                *title.as_ref().as_pcwstr(),
                style,
                x,
                y,
                width,
                height,
                None,
                None,
                Some(instance),
                Some(std::ptr::from_ref(window).cast()),
            )?
        };
        let frame = Self { hwnd };
        frame.apply_custom_frame(window);
        frame.apply_initial_placement(window, saved);
        Ok(frame)
    }

    pub(super) fn show(&self) -> Result<()> {
        // The first ShowWindow call can restore a maximized window according to
        // the launcher's STARTUPINFO. Publish the configured frame unchanged.
        // SAFETY: The window is live; only visibility and frame layout change.
        unsafe {
            WindowsAndMessaging::SetWindowPos(
                self.hwnd,
                None,
                0,
                0,
                0,
                0,
                WindowsAndMessaging::SWP_NOMOVE
                    | WindowsAndMessaging::SWP_NOSIZE
                    | WindowsAndMessaging::SWP_NOZORDER
                    | WindowsAndMessaging::SWP_SHOWWINDOW
                    | WindowsAndMessaging::SWP_FRAMECHANGED,
            )?;
        }
        Ok(())
    }

    /// Keep DWM's shadow and system-managed corners for custom frames.
    fn apply_custom_frame(&self, window: &WindowOptions) {
        if !matches!(window.titlebar, TitlebarStyle::None) {
            return;
        }
        if let Err(error) = extend_frameless_frame(self.hwnd) {
            eprintln!("WebUI: failed to extend the desktop frame: {error}");
        }

        // SAFETY: The window is live; the reposition only forces the frame to
        // be recalculated so `WM_NCCALCSIZE` runs with the new styles.
        unsafe {
            if let Err(error) = WindowsAndMessaging::SetWindowPos(
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
            ) {
                eprintln!("WebUI: failed to refresh the desktop frame: {error}");
            }
        }
    }

    /// Apply initial placement without making the window visible.
    fn apply_initial_placement(&self, window: &WindowOptions, saved: Option<&WindowState>) {
        if saved.is_none() && window.center && !window.maximized {
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
    }
}

impl Drop for FrameWindow {
    fn drop(&mut self) {
        // SAFETY: This owner is dropped on the creating thread before runtime
        // shutdown, including when setup fails before the window is shown.
        unsafe {
            if WindowsAndMessaging::IsWindow(Some(self.hwnd)).as_bool() {
                if let Err(error) = WindowsAndMessaging::DestroyWindow(self.hwnd) {
                    eprintln!("WebUI: failed to destroy the desktop window: {error}");
                }
            }
        }
    }
}

pub(super) fn extend_frameless_frame(hwnd: HWND) -> windows::core::Result<()> {
    // SAFETY: A one-pixel DWM margin preserves the live frameless HWND's
    // shadow/corners without taking ownership of AppWindow overlay frames.
    unsafe {
        windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea(
            hwnd,
            &windows::Win32::UI::Controls::MARGINS {
                cxLeftWidth: 1,
                cxRightWidth: 1,
                cyTopHeight: 1,
                cyBottomHeight: 1,
            },
        )
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
    // Hide the caption in WM_NCCALCSIZE, not in the native capability bits.
    style
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::{LPARAM, RECT, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::WS_CAPTION;

    struct TestFrame(FrameWindow);

    impl Drop for TestFrame {
        fn drop(&mut self) {
            // SAFETY: The test owns this window and destroys it on its UI thread.
            if let Err(error) = unsafe { WindowsAndMessaging::DestroyWindow(self.0.hwnd) } {
                eprintln!("Failed to destroy test window: {error}");
            }
        }
    }

    fn frame_rectangles(frame: &TestFrame) -> (RECT, RECT) {
        let mut outer = RECT::default();
        let mut client = RECT::default();
        // SAFETY: The test window is live and both rectangles are writable.
        unsafe {
            WindowsAndMessaging::GetWindowRect(frame.0.hwnd, &mut outer).unwrap();
            WindowsAndMessaging::GetClientRect(frame.0.hwnd, &mut client).unwrap();
        }
        (outer, client)
    }

    #[test]
    fn startup_custom_frame_reserves_only_native_resize_borders() {
        {
            let titlebar = TitlebarStyle::None;
            let options = WindowOptions {
                titlebar,
                center: false,
                width: 600,
                height: 400,
                ..WindowOptions::default()
            };
            let frame = TestFrame(FrameWindow::new(&options, None).unwrap());
            let (outer, client) = frame_rectangles(&frame);
            let (border_x, border_y) = if matches!(options.titlebar, TitlebarStyle::None) {
                // SAFETY: These calls only read the test window's DPI and metrics.
                unsafe {
                    let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(frame.0.hwnd);
                    let metric =
                        |index| windows::Win32::UI::HiDpi::GetSystemMetricsForDpi(index, dpi);
                    let padded = metric(WindowsAndMessaging::SM_CXPADDEDBORDER);
                    (
                        metric(WindowsAndMessaging::SM_CXFRAME) + padded,
                        metric(WindowsAndMessaging::SM_CYFRAME) + padded,
                    )
                }
            } else {
                (0, 0)
            };

            assert_eq!(
                (client.right - client.left, client.bottom - client.top),
                (
                    outer.right - outer.left - 2 * border_x,
                    outer.bottom - outer.top - 2 * border_y
                ),
                "hide the caption without letting WebView2 cover resize borders: {:?}",
                options.titlebar
            );
            // SAFETY: The test window is live; this only reads visibility.
            assert!(!unsafe { WindowsAndMessaging::IsWindowVisible(frame.0.hwnd) }.as_bool());
        }
    }

    #[test]
    fn startup_native_frame_retains_system_chrome() {
        let frame = TestFrame(
            FrameWindow::new(
                &WindowOptions {
                    center: false,
                    ..WindowOptions::default()
                },
                None,
            )
            .unwrap(),
        );
        let (outer, client) = frame_rectangles(&frame);

        assert!(client.bottom - client.top < outer.bottom - outer.top);
    }

    #[test]
    fn startup_custom_frame_handles_both_non_client_geometry_messages() {
        let frame = TestFrame(
            FrameWindow::new(
                &WindowOptions {
                    titlebar: TitlebarStyle::None,
                    resizable: false,
                    center: false,
                    ..WindowOptions::default()
                },
                None,
            )
            .unwrap(),
        );
        let expected = RECT {
            left: 10,
            top: 20,
            right: 610,
            bottom: 420,
        };
        let mut rect = expected;
        let mut params = WindowsAndMessaging::NCCALCSIZE_PARAMS {
            rgrc: [expected; 3],
            ..Default::default()
        };
        // SAFETY: Both messages synchronously borrow the corresponding writable
        // Win32 geometry type. No pointer escapes SendMessageW.
        unsafe {
            WindowsAndMessaging::SendMessageW(
                frame.0.hwnd,
                WindowsAndMessaging::WM_NCCALCSIZE,
                Some(WPARAM(0)),
                Some(LPARAM(std::ptr::from_mut(&mut rect) as isize)),
            );
            WindowsAndMessaging::SendMessageW(
                frame.0.hwnd,
                WindowsAndMessaging::WM_NCCALCSIZE,
                Some(WPARAM(1)),
                Some(LPARAM(std::ptr::from_mut(&mut params) as isize)),
            );
        }
        assert_eq!(rect, expected);
        assert_eq!(params.rgrc[0], expected);
    }

    #[test]
    fn startup_maximized_frame_stays_hidden_until_ready() {
        let saved = WindowState {
            x: 30,
            y: 40,
            width: 900,
            height: 700,
            maximized: true,
        };
        for (maximized, saved) in [(true, None), (false, Some(&saved))] {
            let frame = TestFrame(
                FrameWindow::new(
                    &WindowOptions {
                        titlebar: TitlebarStyle::None,
                        maximized,
                        center: false,
                        ..WindowOptions::default()
                    },
                    saved,
                )
                .unwrap(),
            );

            // SAFETY: The test window is live; these only read native state.
            unsafe {
                assert!(WindowsAndMessaging::IsZoomed(frame.0.hwnd).as_bool());
                assert!(
                    !WindowsAndMessaging::IsWindowVisible(frame.0.hwnd).as_bool(),
                    "restoring maximized geometry must not show an uninitialized window"
                );
            }
            let (_, client) = frame_rectangles(&frame);
            let work_area = super::super::command::monitor_rect(frame.0.hwnd, true).unwrap();
            assert_eq!(
                (client.right - client.left, client.bottom - client.top),
                (
                    work_area.right - work_area.left,
                    work_area.bottom - work_area.top
                ),
                "maximized custom content must fill, but not exceed, the monitor work area"
            );
            frame.0.show().unwrap();
            // SAFETY: The test owns the live window; these calls only read state.
            unsafe {
                assert!(WindowsAndMessaging::IsWindowVisible(frame.0.hwnd).as_bool());
                assert!(WindowsAndMessaging::IsZoomed(frame.0.hwnd).as_bool());
            }
            assert_eq!(frame_rectangles(&frame).1, client);
        }
    }

    #[test]
    fn frameless_style_preserves_native_frame_capabilities() {
        let style = window_style(&WindowOptions {
            titlebar: TitlebarStyle::None,
            ..WindowOptions::default()
        });

        assert_eq!(style & WS_CAPTION, WS_CAPTION);
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
