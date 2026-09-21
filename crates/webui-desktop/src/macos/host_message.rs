// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Host-message bridge for `[webui-drag]` regions and other in-page requests
//! for native window actions (drag, minimize, toggle-maximize, close).

use crate::DesktopHostMessage;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::NSApplication;
use objc2_foundation::{NSObject, NSObjectProtocol, NSString};
use objc2_web_kit::{WKScriptMessage, WKScriptMessageHandler, WKUserContentController};

/// A valid host command has at most 256 UTF-8 bytes, and therefore no more
/// UTF-16 code units. This cheap native bound precedes any Rust allocation;
/// `DesktopHostMessage::from_json` remains the authoritative byte/command check.
const MAX_MESSAGE_UTF16_UNITS: usize = 256;

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
            if let Some(command) = decode_host_message(&body) {
                run_host_message(command, self.mtm());
            }
        }
    }
);

fn decode_host_message(body: &AnyObject) -> Option<DesktopHostMessage> {
    let text = body.downcast_ref::<NSString>()?;
    let length = text.length();
    if length == 0 || length > MAX_MESSAGE_UTF16_UNITS {
        return None;
    }
    // NSString can contain unpaired surrogates. Copy bounded code units through
    // Foundation's safe getter, then validate without its infallible UTF-8 path.
    let mut units = [0_u16; MAX_MESSAGE_UTF16_UNITS];
    for (index, unit) in units[..length].iter_mut().enumerate() {
        *unit = text.characterAtIndex(index);
    }
    let json = String::from_utf16(&units[..length]).ok()?;
    DesktopHostMessage::from_json(&json).ok()
}

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

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::ptr::NonNull;

    use objc2::rc::autoreleasepool;
    use objc2::{AnyThread, DefinedClass};

    use super::*;

    define_class!(
        // SAFETY: This test object is an NSObject subclass with ordinary Rust ivars.
        #[unsafe(super = NSObject)]
        #[ivars = Cell<usize>]
        struct WebuiHostMessageDescriptionProbe;

        // SAFETY: NSObjectProtocol adds no requirements for this subclass.
        unsafe impl NSObjectProtocol for WebuiHostMessageDescriptionProbe {}

        impl WebuiHostMessageDescriptionProbe {
            #[unsafe(method_id(description))]
            fn description(&self) -> Retained<NSString> {
                self.ivars().set(self.ivars().get() + 1);
                NSString::from_str("\"close\"")
            }
        }
    );

    impl WebuiHostMessageDescriptionProbe {
        fn new() -> Retained<Self> {
            let this = Self::alloc().set_ivars(Cell::new(0));
            // SAFETY: NSObject's initializer has the expected signature for this subclass.
            unsafe { msg_send![super(this), init] }
        }
    }

    #[test]
    fn rejects_objects_without_describing_or_coercing_them() {
        autoreleasepool(|_| {
            let body = WebuiHostMessageDescriptionProbe::new();
            assert!(decode_host_message(&body).is_none());
            assert_eq!(body.ivars().get(), 0);
        });
    }

    #[test]
    fn accepts_only_valid_json_window_commands() {
        autoreleasepool(|_| {
            for command in [
                "\"start-drag\"",
                "\"minimize\"",
                "\"toggle-maximize\"",
                "\"close\"",
            ] {
                assert!(decode_host_message(&NSString::from_str(command)).is_some());
            }
            for command in ["close", "\"unknown\"", "{}", "null", ""] {
                assert!(decode_host_message(&NSString::from_str(command)).is_none());
            }
            assert!(matches!(
                decode_host_message(&NSString::from_str("\"\\u0063lose\"")),
                Some(DesktopHostMessage::Close)
            ));
        });
    }

    #[test]
    fn bounds_native_length_and_preserves_the_authoritative_byte_limit() {
        autoreleasepool(|_| {
            let mut command = " ".repeat(MAX_MESSAGE_UTF16_UNITS - "\"close\"".len());
            command.push_str("\"close\"");
            assert!(DesktopHostMessage::from_json(&command).is_ok());
            assert!(decode_host_message(&NSString::from_str(&command)).is_some());
            command.push(' ');
            assert!(decode_host_message(&NSString::from_str(&command)).is_none());

            let multibyte = NSString::from_str(&format!("\"{}\"", "é".repeat(128)));
            assert!(multibyte.length() < MAX_MESSAGE_UTF16_UNITS);
            assert!(decode_host_message(&multibyte).is_none());
        });
    }

    fn native_utf16(units: &[u16]) -> Retained<NSString> {
        // SAFETY: The slice is valid for its declared length (including zero),
        // and Foundation copies the code units before this initializer returns.
        // NSString permits unpaired surrogates; no Rust UTF-8 conversion occurs.
        unsafe {
            NSString::initWithCharacters_length(
                NSString::alloc(),
                NonNull::from(units).cast(),
                units.len(),
            )
        }
    }

    #[test]
    fn rejects_unpaired_native_utf16_without_panicking() {
        autoreleasepool(|_| {
            let cases: &[&[u16]] = &[
                &[0x0022, 0xd800, 0x0022],
                &[0x0022, 0xdc00, 0x0022],
                &[0xd800],
                &[0xdc00],
            ];
            for units in cases {
                assert!(decode_host_message(&native_utf16(units)).is_none());
            }
        });
    }

    #[test]
    fn handles_native_utf16_empty_strings_and_valid_surrogate_pairs() {
        autoreleasepool(|_| {
            assert!(decode_host_message(&native_utf16(&[])).is_none());
            assert!(
                decode_host_message(&native_utf16(&[0x0022, 0xd83d, 0xde00, 0x0022])).is_none()
            );
            assert!(decode_host_message(&native_utf16(&[0xd83d, 0xde00])).is_none());
        });
    }
}
