// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::cell::Cell;

use anyhow::{Context, Result};
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2;
use windows::Win32::Foundation::{GetLastError, SetLastError, ERROR_SUCCESS, HWND, POINT, RECT};
use windows::Win32::Graphics::{Dwm, Gdi};
use windows::Win32::UI::{Controls::MARGINS, HiDpi, WindowsAndMessaging as wm};

use super::app_sdk::{self, Metrics, Runtime};
mod browser;
use crate::{CaptionButtonSize, TitlebarStyle, WindowOptions};
pub(super) use browser::{resize_browser, BrowserWindow};

pub(super) fn enabled(options: &WindowOptions) -> bool {
    cfg!(feature = "native-dwm-frame")
        && options.caption_button_size == CaptionButtonSize::Standard
        && matches!(
            options.titlebar,
            TitlebarStyle::Overlay { .. } | TitlebarStyle::HiddenInset
        )
}

pub(super) enum WindowFrame {
    Sdk(app_sdk::WindowFrame),
    Native(NativeFrame),
}

pub(super) struct NativeFrame {
    hwnd: HWND,
    minimum_height: u32,
    metrics: Cell<Option<Metrics>>,
    fullscreen: Cell<Option<(i32, RECT)>>,
}

impl WindowFrame {
    pub(super) fn attach(
        runtime: Option<&Runtime>,
        hwnd: HWND,
        options: &WindowOptions,
    ) -> Result<Self> {
        if let Some(runtime) = runtime {
            return Ok(Self::Sdk(app_sdk::WindowFrame::attach(
                runtime, hwnd, options,
            )?));
        }
        let frame = NativeFrame {
            hwnd,
            minimum_height: match options.titlebar {
                TitlebarStyle::Overlay { height } => height,
                _ => 0,
            },
            metrics: Cell::new(None),
            fullscreen: Cell::new(None),
        };
        extend(hwnd, frame.minimum_height)?;
        let value = Self::Native(frame);
        value.refresh(hwnd)?;
        Ok(value)
    }

    pub(super) fn metrics_script(&self) -> Option<String> {
        match self {
            Self::Sdk(frame) => frame.metrics_script(),
            Self::Native(frame) => frame.metrics.get().map(Metrics::script),
        }
    }

    pub(super) fn publish_metrics(&self, webview: &ICoreWebView2) -> Result<()> {
        match self {
            Self::Sdk(frame) => frame.publish_metrics(webview),
            Self::Native(frame) => {
                if let Some(metrics) = frame.metrics.get() {
                    metrics.publish(webview)?;
                }
                Ok(())
            }
        }
    }

    pub(super) fn refresh(&self, hwnd: HWND) -> Result<bool> {
        match self {
            Self::Sdk(frame) => frame.refresh(hwnd),
            Self::Native(frame) => {
                // SAFETY: Read-only inspection of the owned native window.
                if unsafe { wm::IsIconic(hwnd) }.as_bool() {
                    return Ok(false);
                }
                // SAFETY: Read-only access to the owned UI-thread HWND.
                let dpi = unsafe { HiDpi::GetDpiForWindow(hwnd) };
                let metrics = if frame.fullscreen.get().is_some() {
                    Metrics {
                        dpi,
                        ..Default::default()
                    }
                } else {
                    extend(hwnd, frame.minimum_height)?;
                    let caption = caption_rect(hwnd)?;
                    let mut client = RECT::default();
                    // SAFETY: The HWND and writable output RECT remain live.
                    unsafe { wm::GetClientRect(hwnd, &mut client)? };
                    Metrics {
                        left: 0,
                        right: client.right - caption.left,
                        height: caption.bottom,
                        dpi,
                        minimum_height: frame.minimum_height,
                    }
                };
                Ok(frame.metrics.replace(Some(metrics)) != Some(metrics))
            }
        }
    }

    pub(super) fn set_fullscreen(&self, enable: bool) -> Result<()> {
        match self {
            Self::Sdk(frame) => frame.set_fullscreen(enable),
            Self::Native(frame) => frame.set_fullscreen(enable),
        }
    }
}

impl NativeFrame {
    fn set_fullscreen(&self, enable: bool) -> Result<()> {
        if enable == self.fullscreen.get().is_some() {
            return Ok(());
        }
        let previous = self.fullscreen.get();
        let mut current_rect = RECT::default();
        // SAFETY: The UI thread owns this live window and output RECT.
        unsafe { wm::GetWindowRect(self.hwnd, &mut current_rect)? };
        // SAFETY: Read the current style before replacing it.
        let current_style = unsafe { wm::GetWindowLongW(self.hwnd, wm::GWL_STYLE) };
        let (style, rect) = if enable {
            let monitor = super::command::monitor_rect(self.hwnd, false)
                .context("cannot resolve fullscreen monitor")?;
            self.fullscreen.set(Some((current_style, current_rect)));
            (
                (current_style.cast_unsigned() & !wm::WS_OVERLAPPEDWINDOW.0).cast_signed(),
                monitor,
            )
        } else {
            let saved = previous.context("native fullscreen state is missing")?;
            self.fullscreen.set(None);
            saved
        };
        if let Err(error) = apply_geometry(self.hwnd, style, rect) {
            self.fullscreen.set(previous);
            apply_geometry(self.hwnd, current_style, current_rect)
                .with_context(|| format!("native fullscreen failed ({error}) and previous geometry could not be restored"))?;
            return Err(error.into());
        }
        Ok(())
    }
}

fn apply_geometry(hwnd: HWND, style: i32, rect: RECT) -> windows::core::Result<()> {
    // SAFETY: The UI thread owns the live HWND and the complete target geometry.
    unsafe {
        SetLastError(ERROR_SUCCESS);
        wm::SetWindowLongW(hwnd, wm::GWL_STYLE, style);
        if GetLastError() != ERROR_SUCCESS {
            return Err(windows::core::Error::from_thread());
        }
        wm::SetWindowPos(
            hwnd,
            None,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            wm::SWP_NOZORDER | wm::SWP_NOOWNERZORDER | wm::SWP_FRAMECHANGED,
        )
    }
}

fn extend(hwnd: HWND, minimum_height: u32) -> Result<()> {
    // SAFETY: DWM reads the live window and stack-owned margins/policy.
    unsafe {
        let policy = Dwm::DWMNCRP_ENABLED;
        Dwm::DwmSetWindowAttribute(
            hwnd,
            Dwm::DWMWA_NCRENDERING_POLICY,
            std::ptr::from_ref(&policy).cast(),
            u32::try_from(std::mem::size_of_val(&policy))?,
        )?;
        let height = physical_caption_height(minimum_height, HiDpi::GetDpiForWindow(hwnd))?;
        Dwm::DwmExtendFrameIntoClientArea(
            hwnd,
            &MARGINS {
                cyTopHeight: height,
                ..Default::default()
            },
        )?;
    }
    Ok(())
}

fn physical_caption_height(minimum_height: u32, dpi: u32) -> Result<i32> {
    let height = u64::from(minimum_height.max(32)) * u64::from(dpi) / 96;
    i32::try_from(height)
        .context("native caption height exceeds Win32 geometry; reduce the overlay height")
}

pub(super) fn caption_rect(hwnd: HWND) -> Result<RECT> {
    let mut bounds = RECT::default();
    let mut window = RECT::default();
    let mut origin = POINT::default();
    // SAFETY: All outputs are writable and the owned HWND remains live.
    unsafe {
        Dwm::DwmGetWindowAttribute(
            hwnd,
            Dwm::DWMWA_CAPTION_BUTTON_BOUNDS,
            std::ptr::from_mut(&mut bounds).cast(),
            u32::try_from(std::mem::size_of::<RECT>())?,
        )?;
        wm::GetWindowRect(hwnd, &mut window)?;
        Gdi::ClientToScreen(hwnd, &mut origin).ok()?;
    }
    anyhow::ensure!(
        bounds.right > bounds.left && bounds.bottom > bounds.top,
        "DWM native caption bounds are unavailable"
    );
    bounds.left += window.left - origin.x;
    bounds.right += window.left - origin.x;
    bounds.top += window.top - origin.y;
    bounds.bottom += window.top - origin.y;
    Ok(bounds)
}

#[cfg(all(test, feature = "native-dwm-frame"))]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::{LPARAM, WPARAM};

    #[test]
    fn native_caption_glass_is_erased_without_an_application_background() {
        let _com = super::super::initialize_com().unwrap();
        let options = WindowOptions {
            titlebar: TitlebarStyle::Overlay { height: 48 },
            background: None,
            center: false,
            ..WindowOptions::default()
        };
        let frame = super::super::create::FrameWindow::new(&options, None).unwrap();
        super::super::nonclient::prepare_dwm_frame(frame.hwnd).unwrap();
        frame.show().unwrap();
        let _native = WindowFrame::attach(None, frame.hwnd, &options).unwrap();
        // SAFETY: The fixture owns the HWND and its matching temporary device context.
        unsafe {
            let dc = Gdi::GetDC(Some(frame.hwnd));
            assert!(!dc.is_invalid());
            let painted = wm::SendMessageW(
                frame.hwnd,
                wm::WM_ERASEBKGND,
                Some(WPARAM(dc.0.addr())),
                Some(LPARAM(0)),
            );
            assert_eq!(painted.0, 1);
            assert_eq!(Gdi::ReleaseDC(Some(frame.hwnd), dc), 1);
        }
    }

    #[test]
    fn native_frame_selection_preserves_other_caption_backends() {
        let mut options = WindowOptions {
            titlebar: TitlebarStyle::Overlay { height: 48 },
            ..WindowOptions::default()
        };
        assert!(enabled(&options));
        options.caption_button_size = CaptionButtonSize::Tall;
        assert!(!enabled(&options));
        options.caption_button_size = CaptionButtonSize::Standard;
        options.titlebar = TitlebarStyle::Native;
        assert!(!enabled(&options));
        options.titlebar = TitlebarStyle::None;
        assert!(!enabled(&options));
    }

    #[test]
    fn caption_height_scales_without_wrapping_large_configuration_values() {
        assert_eq!(physical_caption_height(48, 144).unwrap(), 72);
        assert_eq!(physical_caption_height(0, 96).unwrap(), 32);
        assert!(physical_caption_height(u32::MAX, 192).is_err());
    }

    #[test]
    fn native_caption_hit_targets_are_excluded_from_the_browser_container() {
        let _com = super::super::initialize_com().unwrap();
        let options = WindowOptions {
            titlebar: TitlebarStyle::Overlay { height: 48 },
            center: false,
            width: 1200,
            height: 800,
            ..WindowOptions::default()
        };
        let frame = super::super::create::FrameWindow::new(&options, None).unwrap();
        super::super::nonclient::prepare_dwm_frame(frame.hwnd).unwrap();
        frame.show().unwrap();
        let native = WindowFrame::attach(None, frame.hwnd, &options).unwrap();
        let browser = BrowserWindow::new(frame.hwnd).unwrap();
        resize_browser(frame.hwnd, browser.hwnd).unwrap();
        let caption = caption_rect(frame.hwnd).unwrap();
        let mut accessible = wm::TITLEBARINFO {
            cbSize: u32::try_from(std::mem::size_of::<wm::TITLEBARINFO>()).unwrap(),
            ..Default::default()
        };
        // SAFETY: The owned native fixture and writable titlebar-info structure are live.
        unsafe { wm::GetTitleBarInfo(frame.hwnd, &mut accessible).unwrap() };
        for button in [2, 3, 5] {
            const STATE_SYSTEM_INVISIBLE: u32 = 0x8000;
            assert_eq!(
                accessible.rgstate[button] & STATE_SYSTEM_INVISIBLE,
                0,
                "native caption accessibility button {button}"
            );
        }
        let mut origin = POINT::default();
        // SAFETY: Read-only coordinate translation for the owned fixture.
        unsafe { Gdi::ClientToScreen(frame.hwnd, &mut origin).ok().unwrap() };
        let width = (caption.right - caption.left) / 3;
        // SAFETY: Copy the owned child's region into a fixture-owned region.
        let region = unsafe { Gdi::CreateRectRgn(0, 0, 0, 0) };
        assert!(!region.is_invalid());
        assert_ne!(
            unsafe { Gdi::GetWindowRgn(browser.hwnd, region) },
            Gdi::RGN_ERROR
        );
        assert!(unsafe { Gdi::PtInRegion(region, 10, 60) }.as_bool());
        for (index, expected) in [(0, wm::HTMINBUTTON), (1, wm::HTMAXBUTTON), (2, wm::HTCLOSE)] {
            let x = origin.x + caption.left + width * index + width / 2;
            let y = origin.y + (caption.top + caption.bottom) / 2;
            let packed =
                u32::from(u16::try_from(x).unwrap()) | (u32::from(u16::try_from(y).unwrap()) << 16);
            // SAFETY: Synchronous read-only hit testing with a valid screen point.
            let hit = unsafe {
                wm::SendMessageW(
                    frame.hwnd,
                    wm::WM_NCHITTEST,
                    Some(WPARAM(0)),
                    Some(LPARAM(isize::try_from(packed).unwrap())),
                )
            };
            assert_eq!(
                hit.0,
                isize::try_from(expected).unwrap(),
                "caption index {index}, bounds {caption:?}"
            );
            assert!(!unsafe { Gdi::PtInRegion(region, x - origin.x, y - origin.y) }.as_bool());
        }
        // SAFETY: The copied test region was never transferred to an HWND.
        unsafe { Gdi::DeleteObject(region.into()).ok().unwrap() };
        native.set_fullscreen(true).unwrap();
        resize_browser(frame.hwnd, browser.hwnd).unwrap();
        let mut fullscreen = RECT::default();
        // SAFETY: Read the client bounds of the owned fullscreen fixture.
        unsafe { wm::GetClientRect(frame.hwnd, &mut fullscreen).unwrap() };
        let monitor = super::super::command::monitor_rect(frame.hwnd, false).unwrap();
        assert_eq!(fullscreen.right, monitor.right - monitor.left);
        assert_eq!(fullscreen.bottom, monitor.bottom - monitor.top);
        native.set_fullscreen(false).unwrap();
        // SAFETY: Read-only state inspection after native presenter restoration.
        assert!(!unsafe { wm::IsZoomed(frame.hwnd) }.as_bool());
    }
}
