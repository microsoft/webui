// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Host-message bridge for `[webui-drag]` regions and other in-page requests
//! for native window actions (drag, minimize, toggle-maximize, close).

use objc2::rc::Retained;
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::NSApplication;
use objc2_foundation::{NSObject, NSObjectProtocol};
use objc2_web_kit::{WKScriptMessage, WKScriptMessageHandler, WKUserContentController};
use webui_desktop::DesktopHostMessage;

#[derive(Debug, Default)]
pub(super) struct HostMessageHandlerIvars;

define_class!(
    // SAFETY: Handler is an NSObject subclass with no Drop implementation.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = HostMessageHandlerIvars]
    pub(super) struct DesktopHostMessageHandler;

    // SAFETY: NSObjectProtocol has no additional safety requirements.
    unsafe impl NSObjectProtocol for DesktopHostMessageHandler {}

    // SAFETY: Method signature matches WKScriptMessageHandler.
    #[allow(non_snake_case)]
    unsafe impl WKScriptMessageHandler for DesktopHostMessageHandler {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        unsafe fn userContentController_didReceiveScriptMessage(
            &self,
            _controller: &WKUserContentController,
            message: &WKScriptMessage,
        ) {
            let body = message.body();
            // SAFETY: Every Objective-C object responds to `description`; WebKit owns
            // the script-message body for the duration of this delegate callback.
            let text: Retained<objc2_foundation::NSString> =
                unsafe { msg_send![&*body, description] };
            if let Ok(command) = DesktopHostMessage::from_json(&text.to_string()) {
                run_host_message(command, self.mtm());
            }
        }
    }
);

fn run_host_message(command: DesktopHostMessage, mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    let Some(window) = app.keyWindow() else {
        return;
    };
    match command {
        DesktopHostMessage::StartDrag => {
            if let Some(event) = app.currentEvent() {
                window.performWindowDragWithEvent(&event);
            }
        }
        DesktopHostMessage::Minimize => window.miniaturize(None),
        DesktopHostMessage::ToggleMaximize => window.zoom(None),
        DesktopHostMessage::Close => window.performClose(None),
    }
}

impl DesktopHostMessageHandler {
    pub(super) fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(HostMessageHandlerIvars);
        // SAFETY: NSObject init has the expected signature for this subclass.
        unsafe { msg_send![super(this), init] }
    }
}
