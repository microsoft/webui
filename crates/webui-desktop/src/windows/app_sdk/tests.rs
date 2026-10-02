// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use super::*;
use crate::windows::create::FrameWindow;
use windows::Win32::UI::HiDpi::GetDpiForWindow;

#[test]
fn caption_button_size_selects_supported_native_sizes() {
    assert_eq!(
        height_option(CaptionButtonSize::Standard),
        TitleBarHeightOption::Standard
    );
    assert_eq!(
        height_option(CaptionButtonSize::Tall),
        TitleBarHeightOption::Tall
    );
}

#[test]
fn native_sdk_attaches_hidden_tall_caption_and_restores_fullscreen() {
    let _bootstrap_lock = super::BOOTSTRAP_TEST_LOCK.lock().unwrap();
    let _com = crate::windows::initialize_com().unwrap();
    let runtime = Runtime::initialize().unwrap();
    let options = WindowOptions {
        titlebar: TitlebarStyle::Overlay { height: 48 },
        caption_button_size: CaptionButtonSize::Tall,
        center: false,
        width: 800,
        height: 600,
        ..WindowOptions::default()
    };
    let frame = FrameWindow::new(&options, None).unwrap();
    let sdk = WindowFrame::attach(&runtime, frame.hwnd, &options).unwrap();
    let caption = sdk.overlay.as_ref().unwrap();
    assert!(caption.titlebar.ExtendsContentIntoTitleBar().unwrap());
    assert_eq!(
        caption.titlebar.PreferredHeightOption().unwrap(),
        TitleBarHeightOption::Tall
    );
    // SAFETY: The test owns this live window and only reads its state.
    unsafe {
        assert!(!WindowsAndMessaging::IsWindowVisible(frame.hwnd).as_bool());
        let dpi = GetDpiForWindow(frame.hwnd);
        assert_eq!(
            caption.titlebar.Height().unwrap(),
            i32::try_from(48 * dpi / 96).unwrap()
        );
    }

    let window = sdk.window.as_ref().unwrap();
    let size = window.Size().unwrap();
    sdk.set_fullscreen(true).unwrap();
    // SAFETY: Changing the presenter must not reveal an uninitialized window.
    assert!(!unsafe { WindowsAndMessaging::IsWindowVisible(frame.hwnd) }.as_bool());
    assert_eq!(
        window.Presenter().unwrap().Kind().unwrap(),
        AppWindowPresenterKind::FullScreen
    );
    sdk.set_fullscreen(false).unwrap();
    assert_eq!(
        window.Presenter().unwrap().Kind().unwrap(),
        AppWindowPresenterKind::Overlapped
    );
    assert_eq!(window.Size().unwrap(), size);
}

#[test]
fn win32_standard_buttons_keep_a_taller_application_bar() {
    use windows::Win32::Foundation::{LPARAM, WPARAM};

    let options = WindowOptions {
        titlebar: TitlebarStyle::Overlay { height: 48 },
        center: false,
        ..WindowOptions::default()
    };
    let frame = FrameWindow::new(&options, None).unwrap();
    let sdk = WindowFrame::without_sdk(frame.hwnd, &options);
    // DWM does not publish live caption hit regions until the frame is visible.
    // SAFETY: The test owns this window and shows it without activation.
    unsafe {
        let _ = WindowsAndMessaging::ShowWindow(frame.hwnd, WindowsAndMessaging::SW_SHOWNOACTIVATE);
        windows::Win32::Graphics::Dwm::DwmFlush().unwrap();
    }
    sdk.refresh(frame.hwnd).unwrap();
    let caption = sdk.native_overlay.as_ref().unwrap();
    let metrics = caption.metrics.get().unwrap();
    assert!(metrics.right > 0);
    assert!(metrics.height > 0);
    assert_eq!(caption.metrics.get().unwrap().minimum_height, 48);

    let mut window_rect = RECT::default();
    let mut client_rect = RECT::default();
    let mut client_origin = POINT::default();
    let mut buttons = RECT::default();
    // SAFETY: The test owns the live HWND and both rectangles are writable.
    unsafe {
        WindowsAndMessaging::GetWindowRect(frame.hwnd, &mut window_rect).unwrap();
        WindowsAndMessaging::GetClientRect(frame.hwnd, &mut client_rect).unwrap();
        windows::Win32::Graphics::Gdi::ClientToScreen(frame.hwnd, &mut client_origin)
            .ok()
            .unwrap();
        DwmGetWindowAttribute(
            frame.hwnd,
            DWMWA_CAPTION_BUTTON_BOUNDS,
            std::ptr::from_mut(&mut buttons).cast(),
            u32::try_from(std::mem::size_of::<RECT>()).unwrap(),
        )
        .unwrap();
    }
    assert_eq!(
        metrics.right,
        client_rect.right - (window_rect.left + buttons.left - client_origin.x)
    );
    let width = buttons.right - buttons.left;
    let y = window_rect.top + (buttons.top + buttons.bottom) / 2;
    for (column, expected) in [
        (0, WindowsAndMessaging::HTMINBUTTON),
        (1, WindowsAndMessaging::HTMAXBUTTON),
        (2, WindowsAndMessaging::HTCLOSE),
    ] {
        let x = window_rect.left + buttons.left + width * (column * 2 + 1) / 6;
        let bits = ((y.cast_unsigned() & 0xffff) << 16) | (x.cast_unsigned() & 0xffff);
        // SAFETY: WM_NCHITTEST synchronously reads the packed screen coordinate.
        let actual = unsafe {
            WindowsAndMessaging::SendMessageW(
                frame.hwnd,
                WindowsAndMessaging::WM_NCHITTEST,
                Some(WPARAM(0)),
                Some(LPARAM(isize::try_from(bits).unwrap())),
            )
            .0
        };
        assert_eq!(actual, isize::try_from(expected).unwrap());
    }
}

#[test]
fn win32_native_controls_preserve_frame_capabilities_and_input_safe_areas() {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    for titlebar in [
        TitlebarStyle::Overlay { height: 48 },
        TitlebarStyle::HiddenInset,
        TitlebarStyle::None,
        TitlebarStyle::Native,
    ] {
        for resizable in [true, false] {
            let options = WindowOptions {
                titlebar: titlebar.clone(),
                resizable,
                center: false,
                ..WindowOptions::default()
            };
            let frame = FrameWindow::new(&options, None).unwrap();
            let sdk = WindowFrame::without_sdk(frame.hwnd, &options);
            sdk.refresh(frame.hwnd).unwrap();
            let mut outer = RECT::default();
            // SAFETY: The fixture owns the HWND and all synchronous message storage.
            let hit = unsafe {
                WindowsAndMessaging::GetWindowRect(frame.hwnd, &mut outer).unwrap();
                let x = outer.left.cast_unsigned() & 0xffff;
                let y = outer.top.cast_unsigned() & 0xffff;
                WindowsAndMessaging::SendMessageW(
                    frame.hwnd,
                    WindowsAndMessaging::WM_NCHITTEST,
                    Some(WPARAM(0)),
                    Some(LPARAM(
                        isize::try_from(((y << 16) | x).cast_signed()).unwrap(),
                    )),
                )
                .0
            };
            let is_resize = (isize::try_from(WindowsAndMessaging::HTLEFT).unwrap()
                ..=isize::try_from(WindowsAndMessaging::HTBOTTOMRIGHT).unwrap())
                .contains(&hit);
            assert_eq!(is_resize, resizable, "{titlebar:?}: hit {hit}");
            if let Some(caption) = &sdk.native_overlay {
                let metrics = caption.metrics.get().unwrap();
                assert!(metrics.right > 0);
                assert!(metrics.height > 0);
            }
            let before = crate::windows::state::window_style_bits(
                frame.hwnd,
                WindowsAndMessaging::GWL_STYLE,
            );
            let before_rect = outer;
            sdk.set_fullscreen(true).unwrap();
            sdk.refresh(frame.hwnd).unwrap();
            if let Some(caption) = &sdk.native_overlay {
                assert_eq!(caption.metrics.get().unwrap().height, 0);
            }
            sdk.set_fullscreen(false).unwrap();
            let after = crate::windows::state::window_style_bits(
                frame.hwnd,
                WindowsAndMessaging::GWL_STYLE,
            );
            let mut after_rect = RECT::default();
            // SAFETY: The test owns the live HWND and rectangle storage.
            unsafe { WindowsAndMessaging::GetWindowRect(frame.hwnd, &mut after_rect).unwrap() };
            assert_eq!(after, before);
            assert_eq!(after_rect, before_rect);
        }
    }
}

#[test]
fn overlay_keeps_all_eight_resize_hit_targets() {
    use windows::Win32::Foundation::{LPARAM, WPARAM};

    let options = WindowOptions {
        titlebar: TitlebarStyle::Overlay { height: 48 },
        center: false,
        width: 800,
        height: 600,
        ..WindowOptions::default()
    };
    let frame = FrameWindow::new(&options, None).unwrap();
    let _sdk = WindowFrame::without_sdk(frame.hwnd, &options);
    let mut rect = RECT::default();
    // SAFETY: This test owns the live window and supplies writable RECT storage.
    unsafe { WindowsAndMessaging::GetWindowRect(frame.hwnd, &mut rect).unwrap() };
    let mid_x = (rect.left + rect.right) / 2;
    let mid_y = (rect.top + rect.bottom) / 2;
    let hit = |x: i32, y: i32| {
        let bits = ((y.cast_unsigned() & 0xffff) << 16) | (x.cast_unsigned() & 0xffff);
        // SAFETY: This test owns the live window; WM_NCHITTEST reads the packed screen point.
        unsafe {
            WindowsAndMessaging::SendMessageW(
                frame.hwnd,
                WindowsAndMessaging::WM_NCHITTEST,
                Some(WPARAM(0)),
                Some(LPARAM(isize::try_from(bits).unwrap())),
            )
            .0
        }
    };
    for ((x, y), expected) in [
        ((rect.left, rect.top), WindowsAndMessaging::HTTOPLEFT),
        ((mid_x, rect.top), WindowsAndMessaging::HTTOP),
        ((rect.right - 1, rect.top), WindowsAndMessaging::HTTOPRIGHT),
        ((rect.left, mid_y), WindowsAndMessaging::HTLEFT),
        ((rect.right - 1, mid_y), WindowsAndMessaging::HTRIGHT),
        (
            (rect.left, rect.bottom - 1),
            WindowsAndMessaging::HTBOTTOMLEFT,
        ),
        ((mid_x, rect.bottom - 1), WindowsAndMessaging::HTBOTTOM),
        (
            (rect.right - 1, rect.bottom - 1),
            WindowsAndMessaging::HTBOTTOMRIGHT,
        ),
    ] {
        assert_eq!(
            hit(x, y),
            isize::try_from(expected).unwrap(),
            "at ({x}, {y})"
        );
    }
}
