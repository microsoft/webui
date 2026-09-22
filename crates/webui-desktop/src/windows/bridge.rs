// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Window controls and typed application IPC. Resources use native interception.

#[cfg(feature = "application-ipc")]
use std::rc::Weak;

use anyhow::Result;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2, ICoreWebView2WebMessageReceivedEventHandler,
};
use webview2_com::WebMessageReceivedEventHandler;
use windows::Win32::Foundation::HWND;

use super::webview::handle_host_message;

pub(super) fn register_message_handler(
    webview: &ICoreWebView2,
    hwnd: HWND,
    #[cfg(feature = "application-ipc")] ipc: Weak<super::ipc::WindowsIpc>,
) -> Result<ICoreWebView2WebMessageReceivedEventHandler> {
    let handler = WebMessageReceivedEventHandler::create(Box::new(move |_sender, args| {
        if let Some(args) = args {
            #[cfg(feature = "application-ipc")]
            if let Some(ipc) = ipc.upgrade() {
                ipc.message(&args)?;
            }
            handle_host_message(hwnd, &args)?;
        }
        Ok(())
    }));
    let mut token = 0_i64;
    // SAFETY: WebView2 retains the handler, and the caller retains its interface
    // for the window lifetime. Application IPC captures only a weak reference.
    unsafe { webview.add_WebMessageReceived(&handler, &mut token)? };
    Ok(handler)
}
