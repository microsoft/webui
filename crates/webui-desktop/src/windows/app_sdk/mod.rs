// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Windows App SDK owns the native caption buttons and non-client input.

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
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging;

use crate::{CaptionButtonSize, TitlebarStyle, WindowOptions};
use bindings::Microsoft::UI::Input::{InputNonClientPointerSource, NonClientRegionKind};
use bindings::Microsoft::UI::Windowing::{
    AppWindow, AppWindowPresenter, AppWindowPresenterKind, AppWindowTitleBar, IconShowOptions,
    TitleBarHeightOption,
};
pub(super) use runtime::Runtime;

pub(super) struct WindowFrame {
    window: AppWindow,
    presenter: AppWindowPresenter,
    overlay: Option<Caption>,
}

struct Caption {
    titlebar: AppWindowTitleBar,
    input: InputNonClientPointerSource,
    metrics: Cell<Option<metrics::Metrics>>,
    region: Cell<Option<RectInt32>>,
    minimum_height: u32,
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
            presenter: window.Presenter()?,
            window,
            overlay,
        };
        frame
            .refresh(hwnd)
            .context("cannot initialize titlebar input regions")?;
        Ok(frame)
    }

    pub(super) fn install_metrics(&self, webview: &ICoreWebView2) -> Result<()> {
        if let Some(metrics) = self
            .overlay
            .as_ref()
            .and_then(|caption| caption.metrics.get())
        {
            crate::windows::webview::add_document_script(webview, &metrics.script())?;
        }
        Ok(())
    }

    pub(super) fn publish_metrics(&self, webview: &ICoreWebView2) -> Result<()> {
        if let Some(metrics) = self
            .overlay
            .as_ref()
            .and_then(|caption| caption.metrics.get())
        {
            metrics.publish(webview)?;
        }
        Ok(())
    }

    pub(super) fn set_fullscreen(&self, enable: bool) -> Result<()> {
        if enable {
            self.window
                .SetPresenterByKind(AppWindowPresenterKind::FullScreen)?;
        } else {
            self.window.SetPresenter(&self.presenter)?;
        }
        Ok(())
    }

    pub(super) fn refresh(&self, hwnd: HWND) -> Result<bool> {
        let Some(caption) = &self.overlay else {
            return Ok(false);
        };
        let mut client = RECT::default();
        // SAFETY: The HWND is live and the rectangle is writable.
        unsafe { WindowsAndMessaging::GetClientRect(hwnd, &mut client)? };
        let left = caption.titlebar.LeftInset()?;
        let right = caption.titlebar.RightInset()?;
        let fullscreen = self.window.Presenter()?.Kind()? == AppWindowPresenterKind::FullScreen;
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
}

fn height_option(size: CaptionButtonSize) -> TitleBarHeightOption {
    match size {
        CaptionButtonSize::Standard => TitleBarHeightOption::Standard,
        CaptionButtonSize::Tall => TitleBarHeightOption::Tall,
    }
}
