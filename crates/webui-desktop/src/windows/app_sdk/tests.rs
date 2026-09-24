// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use super::*;
use crate::windows::create::FrameWindow;
use windows::Win32::UI::HiDpi::GetDpiForWindow;

#[test]
fn height_preference_uses_supported_native_sizes() {
    assert_eq!(
        height_option(&TitlebarStyle::Overlay { height: 32 }),
        TitleBarHeightOption::Standard
    );
    assert_eq!(
        height_option(&TitlebarStyle::Overlay { height: 48 }),
        TitleBarHeightOption::Tall
    );
    assert_eq!(
        height_option(&TitlebarStyle::HiddenInset),
        TitleBarHeightOption::Standard
    );
}

#[test]
fn native_sdk_attaches_hidden_tall_caption_and_restores_fullscreen() {
    let _bootstrap_lock = super::BOOTSTRAP_TEST_LOCK.lock().unwrap();
    let _com = crate::windows::initialize_com().unwrap();
    let runtime = Runtime::initialize().unwrap();
    let options = WindowOptions {
        titlebar: TitlebarStyle::Overlay { height: 48 },
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
    let size = sdk.window.Size().unwrap();
    sdk.set_fullscreen(true).unwrap();
    // SAFETY: Changing the presenter must not reveal an uninitialized window.
    assert!(!unsafe { WindowsAndMessaging::IsWindowVisible(frame.hwnd) }.as_bool());
    assert_eq!(
        sdk.window.Presenter().unwrap().Kind().unwrap(),
        AppWindowPresenterKind::FullScreen
    );
    sdk.set_fullscreen(false).unwrap();
    assert_eq!(
        sdk.window.Presenter().unwrap().Kind().unwrap(),
        AppWindowPresenterKind::Overlapped
    );
    assert_eq!(sdk.window.Size().unwrap(), size);
}

#[test]
fn sdk_native_controls_preserve_frame_capabilities_and_input_safe_areas() {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    let _bootstrap_lock = super::BOOTSTRAP_TEST_LOCK.lock().unwrap();
    let _com = crate::windows::initialize_com().unwrap();
    let runtime = Runtime::initialize().unwrap();
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
            let sdk = WindowFrame::attach(&runtime, frame.hwnd, &options).unwrap();
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
            if let Some(caption) = &sdk.overlay {
                let metrics = caption.metrics.get().unwrap();
                assert!(metrics.left + metrics.right > 0);
                let regions = caption
                    .input
                    .GetRegionRects(NonClientRegionKind::Passthrough)
                    .unwrap();
                assert_eq!(regions.len(), 1);
                assert_eq!(regions[0].X, metrics.left);
                assert_eq!(regions[0].Height, metrics.height);
                assert_eq!(
                    regions[0].X + regions[0].Width + metrics.right,
                    sdk.window.ClientSize().unwrap().Width
                );
            }
            let before = crate::windows::state::window_style_bits(
                frame.hwnd,
                WindowsAndMessaging::GWL_STYLE,
            );
            sdk.set_fullscreen(true).unwrap();
            sdk.refresh(frame.hwnd).unwrap();
            if let Some(caption) = &sdk.overlay {
                assert_eq!(caption.metrics.get().unwrap().height, 0);
            }
            sdk.set_fullscreen(false).unwrap();
            let after = crate::windows::state::window_style_bits(
                frame.hwnd,
                WindowsAndMessaging::GWL_STYLE,
            );
            let capabilities =
                WindowsAndMessaging::WS_THICKFRAME.0 | WindowsAndMessaging::WS_MAXIMIZEBOX.0;
            assert_eq!(before & capabilities, after & capabilities);
        }
    }
}
