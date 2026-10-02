// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Native Windows frame backends and custom-titlebar safe-area metrics.

mod bindings;
mod metrics;
mod runtime;
#[cfg(test)]
mod tests;

// MddBootstrap initialization and shutdown affect the entire test process.
#[cfg(test)]
static BOOTSTRAP_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

use anyhow::{Context, Result};
use std::cell::Cell;
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2;
use windows::Graphics::RectInt32;
use windows::Win32::Foundation::{GetLastError, SetLastError, ERROR_SUCCESS, HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CAPTION_BUTTON_BOUNDS};
use windows::Win32::UI::HiDpi;
use windows::Win32::UI::WindowsAndMessaging;

use crate::{CaptionButtonSize, TitlebarStyle, WindowOptions};
use bindings::Microsoft::UI::Input::{InputNonClientPointerSource, NonClientRegionKind};
use bindings::Microsoft::UI::Windowing::{
    AppWindow, AppWindowPresenter, AppWindowPresenterKind, AppWindowTitleBar, IconShowOptions,
    TitleBarHeightOption,
};
pub(super) use runtime::Runtime;

pub(super) struct WindowFrame {
    hwnd: HWND,
    window: Option<AppWindow>,
    presenter: Option<AppWindowPresenter>,
    overlay: Option<Caption>,
    native_overlay: Option<NativeCaption>,
    native_fullscreen: Cell<Option<NativeFullscreen>>,
}

struct Caption {
    titlebar: AppWindowTitleBar,
    input: InputNonClientPointerSource,
    metrics: Cell<Option<metrics::Metrics>>,
    region: Cell<Option<RectInt32>>,
    minimum_height: u32,
}

struct NativeCaption {
    metrics: Cell<Option<metrics::Metrics>>,
    minimum_height: u32,
}

#[derive(Clone, Copy)]
struct NativeFullscreen {
    style: i32,
    rect: RECT,
}

pub(super) fn requires_runtime(options: &WindowOptions) -> bool {
    options.caption_button_size == CaptionButtonSize::Tall
        && matches!(
            options.titlebar,
            TitlebarStyle::Overlay { .. } | TitlebarStyle::HiddenInset
        )
}

impl WindowFrame {
    pub(super) fn attach(runtime: &Runtime, hwnd: HWND, options: &WindowOptions) -> Result<Self> {
        let id = runtime
            .window_id(hwnd)
            .context("cannot map HWND to WindowId")?;
        let window = AppWindow::GetFromWindowId(id).context("cannot attach AppWindow to HWND")?;
        window
            .AssociateWithDispatcherQueue(runtime.dispatcher())
            .context("cannot associate AppWindow with its UI dispatcher")?;
        let overlay = if matches!(
            options.titlebar,
            TitlebarStyle::Overlay { .. } | TitlebarStyle::HiddenInset
        ) {
            let titlebar = window
                .TitleBar()
                .context("cannot acquire AppWindow titlebar")?;
            titlebar
                .SetExtendsContentIntoTitleBar(true)
                .context("cannot extend content into native titlebar")?;
            titlebar
                .SetPreferredHeightOption(height_option(options.caption_button_size))
                .context("cannot set native titlebar height")?;
            titlebar
                .SetIconShowOptions(IconShowOptions::HideIconAndSystemMenu)
                .context("cannot hide native titlebar icon")?;
            Some(Caption {
                titlebar,
                input: InputNonClientPointerSource::GetForWindowId(id)
                    .context("cannot acquire native non-client input source")?,
                metrics: Cell::new(None),
                region: Cell::new(None),
                minimum_height: match options.titlebar {
                    TitlebarStyle::Overlay { height } => height,
                    _ => 0,
                },
            })
        } else {
            None
        };
        let frame = Self {
            hwnd,
            presenter: Some(window.Presenter()?),
            window: Some(window),
            overlay,
            native_overlay: None,
            native_fullscreen: Cell::new(None),
        };
        frame
            .refresh(hwnd)
            .context("cannot initialize titlebar input regions")?;
        Ok(frame)
    }

    pub(super) fn without_sdk(hwnd: HWND, options: &WindowOptions) -> Self {
        let native_overlay = matches!(
            options.titlebar,
            TitlebarStyle::Overlay { .. } | TitlebarStyle::HiddenInset
        )
        .then(|| NativeCaption {
            metrics: Cell::new(None),
            minimum_height: match options.titlebar {
                TitlebarStyle::Overlay { height } => height,
                _ => 0,
            },
        });
        Self {
            hwnd,
            window: None,
            presenter: None,
            overlay: None,
            native_overlay,
            native_fullscreen: Cell::new(None),
        }
    }

    pub(super) fn metrics_script(&self) -> Option<String> {
        self.overlay
            .as_ref()
            .and_then(|caption| caption.metrics.get())
            .or_else(|| {
                self.native_overlay
                    .as_ref()
                    .and_then(|caption| caption.metrics.get())
            })
            .map(metrics::Metrics::script)
    }

    pub(super) fn publish_metrics(&self, webview: &ICoreWebView2) -> Result<()> {
        if let Some(metrics) = self
            .overlay
            .as_ref()
            .and_then(|caption| caption.metrics.get())
            .or_else(|| {
                self.native_overlay
                    .as_ref()
                    .and_then(|caption| caption.metrics.get())
            })
        {
            metrics.publish(webview)?;
        }
        Ok(())
    }

    pub(super) fn set_fullscreen(&self, enable: bool) -> Result<()> {
        if let Some(window) = &self.window {
            if enable {
                window.SetPresenterByKind(AppWindowPresenterKind::FullScreen)?;
            } else {
                let presenter = self
                    .presenter
                    .as_ref()
                    .context("Windows App SDK presenter is unavailable")?;
                window.SetPresenter(presenter)?;
            }
            return Ok(());
        }
        self.set_native_fullscreen(enable)
    }

    pub(super) fn refresh(&self, hwnd: HWND) -> Result<bool> {
        if let Some(caption) = &self.native_overlay {
            let metrics = native_caption_metrics(hwnd, caption.minimum_height);
            return Ok(caption.metrics.replace(Some(metrics)) != Some(metrics));
        }
        let Some(caption) = &self.overlay else {
            return Ok(false);
        };
        let mut client = RECT::default();
        // SAFETY: The HWND is live and the rectangle is writable.
        unsafe { WindowsAndMessaging::GetClientRect(hwnd, &mut client)? };
        let left = caption.titlebar.LeftInset()?;
        let right = caption.titlebar.RightInset()?;
        let fullscreen = self
            .window
            .as_ref()
            .context("Windows App SDK window is unavailable")?
            .Presenter()?
            .Kind()?
            == AppWindowPresenterKind::FullScreen;
        let height = if fullscreen {
            0
        } else {
            caption.titlebar.Height()?
        };
        let region = RectInt32 {
            X: left,
            Y: 0,
            Width: (client.right - left - right).max(0),
            Height: height,
        };
        if caption.region.get() != Some(region) {
            caption
                .input
                .SetRegionRects(NonClientRegionKind::Passthrough, &[region])?;
            caption.region.set(Some(region));
        }

        // SAFETY: Reading the DPI of this live HWND has no side effects.
        let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd) };
        let metrics = metrics::Metrics {
            left: if fullscreen { 0 } else { left },
            right: if fullscreen { 0 } else { right },
            height,
            dpi,
            minimum_height: if fullscreen {
                0
            } else {
                caption.minimum_height
            },
        };
        Ok(caption.metrics.replace(Some(metrics)) != Some(metrics))
    }

    fn set_native_fullscreen(&self, enable: bool) -> Result<()> {
        if enable {
            if self.native_fullscreen.get().is_some() {
                return Ok(());
            }
            let mut rect = RECT::default();
            // SAFETY: The HWND is live and `rect` is writable storage.
            unsafe { WindowsAndMessaging::GetWindowRect(self.hwnd, &mut rect)? };
            // SAFETY: Reading the style of this live window has no side effects.
            let style = unsafe {
                WindowsAndMessaging::GetWindowLongW(self.hwnd, WindowsAndMessaging::GWL_STYLE)
            };
            let Some(monitor) = super::command::monitor_rect(self.hwnd, false) else {
                anyhow::bail!("cannot resolve the fullscreen monitor");
            };
            self.native_fullscreen
                .set(Some(NativeFullscreen { style, rect }));
            let fullscreen_style =
                (style.cast_unsigned() & !WindowsAndMessaging::WS_OVERLAPPEDWINDOW.0).cast_signed();
            if let Err(error) = set_window_style(self.hwnd, fullscreen_style).and_then(|()| {
                // SAFETY: The monitor rectangle and HWND remain valid for this call.
                unsafe {
                    WindowsAndMessaging::SetWindowPos(
                        self.hwnd,
                        None,
                        monitor.left,
                        monitor.top,
                        monitor.right.saturating_sub(monitor.left),
                        monitor.bottom.saturating_sub(monitor.top),
                        WindowsAndMessaging::SWP_NOZORDER
                            | WindowsAndMessaging::SWP_NOOWNERZORDER
                            | WindowsAndMessaging::SWP_FRAMECHANGED,
                    )
                }
            }) {
                self.native_fullscreen.set(None);
                let _ = set_window_style(self.hwnd, style);
                return Err(error.into());
            }
            return Ok(());
        }

        let Some(previous) = self.native_fullscreen.take() else {
            return Ok(());
        };
        if let Err(error) = set_window_style(self.hwnd, previous.style).and_then(|()| {
            // SAFETY: The saved rectangle came from this HWND immediately
            // before entering fullscreen.
            unsafe {
                WindowsAndMessaging::SetWindowPos(
                    self.hwnd,
                    None,
                    previous.rect.left,
                    previous.rect.top,
                    previous.rect.right.saturating_sub(previous.rect.left),
                    previous.rect.bottom.saturating_sub(previous.rect.top),
                    WindowsAndMessaging::SWP_NOZORDER
                        | WindowsAndMessaging::SWP_NOOWNERZORDER
                        | WindowsAndMessaging::SWP_FRAMECHANGED,
                )
            }
        }) {
            self.native_fullscreen.set(Some(previous));
            return Err(error.into());
        }
        Ok(())
    }
}

fn native_caption_metrics(hwnd: HWND, minimum_height: u32) -> metrics::Metrics {
    let mut bounds = RECT::default();
    // SAFETY: `bounds` is writable and the HWND remains live for this call.
    let bounds_available = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_BUTTON_BOUNDS,
            std::ptr::from_mut(&mut bounds).cast(),
            u32::try_from(std::mem::size_of::<RECT>()).unwrap_or(16),
        )
        .is_ok()
    };
    // SAFETY: These calls only read state from the live HWND.
    let dpi = unsafe { HiDpi::GetDpiForWindow(hwnd) };
    let fullscreen =
        unsafe { WindowsAndMessaging::GetWindowLongW(hwnd, WindowsAndMessaging::GWL_STYLE) }
            .cast_unsigned()
            & WindowsAndMessaging::WS_CAPTION.0
            == 0;
    if fullscreen {
        return metrics::Metrics {
            dpi,
            ..Default::default()
        };
    }
    let (left, right, height) = if bounds_available {
        native_caption_insets(hwnd, bounds).unwrap_or_else(|| fallback_caption_metrics(dpi))
    } else {
        fallback_caption_metrics(dpi)
    };
    metrics::Metrics {
        left,
        right,
        height,
        dpi,
        minimum_height,
    }
}

fn native_caption_insets(hwnd: HWND, bounds: RECT) -> Option<(i32, i32, i32)> {
    let mut window = RECT::default();
    let mut client = RECT::default();
    let mut client_origin = POINT::default();
    // SAFETY: The HWND is live and all output structures are writable.
    unsafe {
        WindowsAndMessaging::GetWindowRect(hwnd, &mut window).ok()?;
        WindowsAndMessaging::GetClientRect(hwnd, &mut client).ok()?;
        windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut client_origin)
            .ok()
            .ok()?;
    }
    let width = client.right.saturating_sub(client.left);
    let caption_left = window
        .left
        .saturating_add(bounds.left)
        .saturating_sub(client_origin.x)
        .clamp(0, width);
    let caption_right = window
        .left
        .saturating_add(bounds.right)
        .saturating_sub(client_origin.x)
        .clamp(0, width);
    let height = window
        .top
        .saturating_add(bounds.bottom)
        .saturating_sub(client_origin.y)
        .max(0);
    if caption_left.saturating_add(caption_right) <= width {
        Some((caption_right, 0, height))
    } else {
        Some((0, width.saturating_sub(caption_left), height))
    }
}

fn fallback_caption_metrics(dpi: u32) -> (i32, i32, i32) {
    // SAFETY: Per-DPI system metrics require no mutable process state.
    unsafe {
        (
            0,
            HiDpi::GetSystemMetricsForDpi(WindowsAndMessaging::SM_CXSIZE, dpi).saturating_mul(3),
            HiDpi::GetSystemMetricsForDpi(WindowsAndMessaging::SM_CYSIZE, dpi),
        )
    }
}

fn set_window_style(hwnd: HWND, style: i32) -> windows::core::Result<()> {
    // SAFETY: The HWND is live and GWL_STYLE accepts this complete style value.
    unsafe {
        SetLastError(ERROR_SUCCESS);
        WindowsAndMessaging::SetWindowLongW(hwnd, WindowsAndMessaging::GWL_STYLE, style);
        if GetLastError() != ERROR_SUCCESS {
            return Err(windows::core::Error::from_thread());
        }
    }
    Ok(())
}

fn height_option(size: CaptionButtonSize) -> TitleBarHeightOption {
    match size {
        CaptionButtonSize::Standard => TitleBarHeightOption::Standard,
        CaptionButtonSize::Tall => TitleBarHeightOption::Tall,
    }
}

#[cfg(test)]
mod runtime_selection_tests {
    use super::*;

    #[test]
    fn only_tall_custom_titlebars_require_the_app_sdk_runtime() {
        for titlebar in [
            TitlebarStyle::Overlay { height: 48 },
            TitlebarStyle::HiddenInset,
        ] {
            assert!(!requires_runtime(&WindowOptions {
                titlebar: titlebar.clone(),
                ..WindowOptions::default()
            }));
            assert!(requires_runtime(&WindowOptions {
                titlebar,
                caption_button_size: CaptionButtonSize::Tall,
                ..WindowOptions::default()
            }));
        }
        for titlebar in [TitlebarStyle::Native, TitlebarStyle::None] {
            assert!(!requires_runtime(&WindowOptions {
                titlebar,
                caption_button_size: CaptionButtonSize::Tall,
                ..WindowOptions::default()
            }));
        }
    }
}
