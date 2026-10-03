// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::WindowOptions;
use anyhow::{Context, Result};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::{HiDpi, WindowsAndMessaging as wm};

pub(in crate::windows) fn prepare_startup_window(
    hwnd: HWND,
    options: &WindowOptions,
    has_saved_geometry: bool,
) -> Result<()> {
    super::super::nonclient::prepare_dwm_frame(hwnd)?;
    if has_saved_geometry || options.maximized || options.fullscreen {
        return Ok(());
    }
    resize_client(hwnd, options.width, options.height)?;
    if options.center {
        super::super::command::center_window(hwnd);
    }
    Ok(())
}

pub(in crate::windows) fn resize_client(
    hwnd: HWND,
    logical_width: u32,
    logical_height: u32,
) -> Result<()> {
    // SAFETY: Read-only measurements of the owned native window.
    let (dpi, has_caption, iconic, zoomed) = unsafe {
        (
            HiDpi::GetDpiForWindow(hwnd),
            wm::GetWindowLongW(hwnd, wm::GWL_STYLE).cast_unsigned() & wm::WS_CAPTION.0 != 0,
            wm::IsIconic(hwnd).as_bool(),
            wm::IsZoomed(hwnd).as_bool(),
        )
    };
    let width = i32::try_from(u64::from(logical_width) * u64::from(dpi) / 96)
        .context("native viewport width exceeds Win32 geometry; reduce the window width")?;
    let height = i32::try_from(u64::from(logical_height) * u64::from(dpi) / 96)
        .context("native viewport height exceeds Win32 geometry; reduce the window height")?;
    // Match FRAME_OVERLAY_DWM's restored client layout. Reading a minimized
    // or maximized client rectangle cannot determine its restored insets.
    let (frame_x, frame_y) = if has_caption {
        super::super::nonclient::frame_thickness(hwnd)
    } else {
        (0, 0)
    };
    let width = width
        .checked_add(frame_x * 2)
        .context("native window width exceeds Win32 geometry; reduce the viewport width")?;
    let height = height
        .checked_add(frame_y)
        .context("native window height exceeds Win32 geometry; reduce the viewport height")?;
    // SAFETY: The owned HWND and target geometry remain live on this UI thread.
    unsafe {
        if iconic || zoomed {
            let mut placement = wm::WINDOWPLACEMENT {
                length: u32::try_from(std::mem::size_of::<wm::WINDOWPLACEMENT>())?,
                ..Default::default()
            };
            wm::GetWindowPlacement(hwnd, &mut placement)?;
            placement.rcNormalPosition.right =
                placement
                    .rcNormalPosition
                    .left
                    .checked_add(width)
                    .context("native restored width exceeds Win32 geometry")?;
            placement.rcNormalPosition.bottom = placement
                .rcNormalPosition
                .top
                .checked_add(height)
                .context("native restored height exceeds Win32 geometry")?;
            wm::SetWindowPlacement(hwnd, &placement)?;
        } else {
            wm::SetWindowPos(
                hwnd,
                None,
                0,
                0,
                width,
                height,
                wm::SWP_NOMOVE | wm::SWP_NOZORDER | wm::SWP_NOACTIVATE | wm::SWP_FRAMECHANGED,
            )?;
        }
    }
    Ok(())
}

#[cfg(all(test, feature = "native-dwm-frame"))]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::windows::{command, create::FrameWindow};
    use crate::{TitlebarStyle, WindowCommand};
    use windows::Win32::Foundation::RECT;

    #[test]
    fn configured_client_viewport_is_ready_before_the_browser_starts() {
        let options = WindowOptions {
            titlebar: TitlebarStyle::Overlay { height: 48 },
            width: 1200,
            height: 800,
            center: false,
            ..WindowOptions::default()
        };
        let frame = FrameWindow::new(&options, None).unwrap();
        prepare_startup_window(frame.hwnd, &options, false).unwrap();
        frame.show().unwrap();
        let mut client = RECT::default();
        // SAFETY: Read-only measurements of the fixture's owned HWND.
        unsafe {
            wm::GetClientRect(frame.hwnd, &mut client).unwrap();
            let dpi = HiDpi::GetDpiForWindow(frame.hwnd);
            assert_eq!(
                client.right,
                i32::try_from(u64::from(options.width) * u64::from(dpi) / 96).unwrap()
            );
            assert_eq!(
                client.bottom,
                i32::try_from(u64::from(options.height) * u64::from(dpi) / 96).unwrap()
            );
        }
        command::execute_window_command(
            frame.hwnd,
            WindowCommand::SetSize {
                width: 640,
                height: 480,
            },
        );
        // SAFETY: The owned frame stays live after the synchronous queued command.
        unsafe {
            wm::GetClientRect(frame.hwnd, &mut client).unwrap();
            let dpi = HiDpi::GetDpiForWindow(frame.hwnd);
            assert_eq!(
                client.right,
                i32::try_from(640 * u64::from(dpi) / 96).unwrap()
            );
            assert_eq!(
                client.bottom,
                i32::try_from(480 * u64::from(dpi) / 96).unwrap()
            );
        }
    }

    #[test]
    fn client_resize_preserves_minimized_and_maximized_state() {
        for show in [wm::SW_MINIMIZE, wm::SW_MAXIMIZE] {
            let options = WindowOptions {
                titlebar: TitlebarStyle::Overlay { height: 48 },
                center: false,
                ..WindowOptions::default()
            };
            let frame = FrameWindow::new(&options, None).unwrap();
            prepare_startup_window(frame.hwnd, &options, false).unwrap();
            frame.show().unwrap();
            // SAFETY: The fixture owns this native window and its show transitions.
            unsafe {
                let _ = wm::ShowWindow(frame.hwnd, show);
            }
            resize_client(frame.hwnd, 640, 480).unwrap();
            // SAFETY: Read-only state checks, then restore the owned fixture.
            unsafe {
                assert_eq!(wm::IsIconic(frame.hwnd).as_bool(), show == wm::SW_MINIMIZE);
                assert_eq!(wm::IsZoomed(frame.hwnd).as_bool(), show == wm::SW_MAXIMIZE);
                let _ = wm::ShowWindow(frame.hwnd, wm::SW_RESTORE);
                let mut client = RECT::default();
                wm::GetClientRect(frame.hwnd, &mut client).unwrap();
                let dpi = HiDpi::GetDpiForWindow(frame.hwnd);
                assert_eq!(
                    client.right,
                    i32::try_from(640 * u64::from(dpi) / 96).unwrap()
                );
                assert_eq!(
                    client.bottom,
                    i32::try_from(480 * u64::from(dpi) / 96).unwrap()
                );
            }
        }
    }

    #[test]
    fn startup_preserves_saved_native_window_bounds() {
        let options = WindowOptions {
            titlebar: TitlebarStyle::Overlay { height: 48 },
            center: false,
            ..WindowOptions::default()
        };
        let frame = FrameWindow::new(&options, None).unwrap();
        let mut before = RECT::default();
        let mut after = RECT::default();
        // SAFETY: Read the bounds of the owned fixture before explicit native layout activation.
        unsafe { wm::GetWindowRect(frame.hwnd, &mut before).unwrap() };
        prepare_startup_window(frame.hwnd, &options, true).unwrap();
        frame.show().unwrap();
        // SAFETY: Read-only bounds inspection of the same owned window.
        unsafe { wm::GetWindowRect(frame.hwnd, &mut after).unwrap() };
        assert_eq!(before, after);
    }
}
