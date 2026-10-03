// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use anyhow::Result;
use windows::core::w;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi;
use windows::Win32::UI::WindowsAndMessaging as wm;

pub(in crate::windows) struct BrowserWindow {
    pub(in crate::windows) hwnd: HWND,
}

impl BrowserWindow {
    pub(in crate::windows) fn new(parent: HWND) -> Result<Self> {
        // SAFETY: The standard STATIC class creates an owned child of the live frame.
        let hwnd = unsafe {
            wm::CreateWindowExW(
                Default::default(),
                w!("STATIC"),
                None,
                wm::WS_CHILD | wm::WS_VISIBLE | wm::WS_CLIPCHILDREN | wm::WS_CLIPSIBLINGS,
                0,
                0,
                0,
                0,
                Some(parent),
                None,
                None,
                None,
            )?
        };
        Ok(Self { hwnd })
    }
}

impl Drop for BrowserWindow {
    fn drop(&mut self) {
        // SAFETY: This owner stays UI-thread-local. Parent destruction may have already released it.
        unsafe {
            if wm::IsWindow(Some(self.hwnd)).as_bool() {
                if let Err(error) = wm::DestroyWindow(self.hwnd) {
                    eprintln!("WebUI: failed to destroy native browser container: {error}");
                }
            }
        }
    }
}

pub(in crate::windows) fn resize_browser(parent: HWND, browser: HWND) -> Result<()> {
    // SAFETY: A minimized native window has no drawable client viewport.
    if unsafe { wm::IsIconic(parent) }.as_bool() {
        return Ok(());
    }
    let mut client = RECT::default();
    // SAFETY: Parent and browser container are live on this UI thread.
    unsafe {
        wm::GetClientRect(parent, &mut client)?;
        wm::SetWindowPos(
            browser,
            None,
            0,
            0,
            client.right,
            client.bottom,
            wm::SWP_NOZORDER | wm::SWP_NOACTIVATE,
        )?;
        if wm::GetWindowLongW(parent, wm::GWL_STYLE).cast_unsigned() & wm::WS_CAPTION.0 == 0 {
            anyhow::ensure!(
                Gdi::SetWindowRgn(browser, None, true) != 0,
                "cannot clear fullscreen browser region"
            );
            return Ok(());
        }
    }
    let caption = super::caption_rect(parent)?;
    // SAFETY: GDI owns both regions; success transfers only the final region to the child HWND.
    unsafe {
        let full = Gdi::CreateRectRgn(0, 0, client.right, client.bottom);
        let cut = Gdi::CreateRectRgn(caption.left, caption.top, caption.right, caption.bottom);
        if full.is_invalid() || cut.is_invalid() {
            if !full.is_invalid() {
                let _ = Gdi::DeleteObject(full.into());
            }
            if !cut.is_invalid() {
                let _ = Gdi::DeleteObject(cut.into());
            }
            anyhow::bail!("cannot allocate native caption exclusion regions");
        }
        let combined = Gdi::CombineRgn(Some(full), Some(full), Some(cut), Gdi::RGN_DIFF);
        let _ = Gdi::DeleteObject(cut.into());
        if combined == Gdi::RGN_ERROR {
            let _ = Gdi::DeleteObject(full.into());
            anyhow::bail!("cannot combine native caption exclusion regions");
        }
        if Gdi::SetWindowRgn(browser, Some(full), true) == 0 {
            let _ = Gdi::DeleteObject(full.into());
            anyhow::bail!("cannot apply native caption exclusion region");
        }
    }
    Ok(())
}
