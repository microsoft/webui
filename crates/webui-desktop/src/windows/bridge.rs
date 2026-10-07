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
    controls: bool,
    #[cfg(feature = "local-server")] local_controls: Option<
        std::rc::Rc<super::local_controls::LocalControls>,
    >,
    #[cfg(feature = "application-ipc")] ipc: Weak<super::ipc::WindowsIpc>,
) -> Result<ICoreWebView2WebMessageReceivedEventHandler> {
    #[cfg(feature = "local-server")]
    let view = webview.clone();
    let handler = WebMessageReceivedEventHandler::create(Box::new(move |_sender, args| {
        if let Some(args) = args {
            #[cfg(feature = "application-ipc")]
            if let Some(ipc) = ipc.upgrade() {
                ipc.message(&args)?;
            }
            if controls {
                handle_host_message(hwnd, &args)?;
            }
            #[cfg(feature = "local-server")]
            if let Some(local) = &local_controls {
                local.message(&view, hwnd, &args)?;
            }
        }
        Ok(())
    }));
    let mut token = 0_i64;
    // SAFETY: WebView2 retains the handler, and the caller retains its interface
    // for the window lifetime. Application IPC captures only a weak reference.
    unsafe { webview.add_WebMessageReceived(&handler, &mut token)? };
    Ok(handler)
}
