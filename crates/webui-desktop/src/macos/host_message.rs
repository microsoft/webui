// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Host-message bridge for `[webui-drag]` regions and other in-page requests
//! for native window actions (drag, minimize, toggle-maximize, close).

use crate::DesktopHostMessage;
#[cfg(feature = "local-server")]
use block2::RcBlock;
use objc2::rc::Retained;
#[cfg(feature = "local-server")]
use objc2::rc::Weak;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSWindow};
#[cfg(feature = "local-server")]
use objc2_foundation::NSError;
use objc2_foundation::{ns_string, NSObject, NSObjectProtocol, NSString, NSURL};
#[cfg(feature = "local-server")]
use objc2_web_kit::WKWebView;
use objc2_web_kit::{WKScriptMessage, WKScriptMessageHandler, WKUserContentController};
#[cfg(feature = "local-server")]
use std::{cell::RefCell, rc::Rc};

/// A valid host command has at most 256 UTF-8 bytes, and therefore no more
/// UTF-16 code units. This cheap native bound precedes any Rust allocation;
/// `DesktopHostMessage::from_json` remains the authoritative byte/command check.
const MAX_MESSAGE_UTF16_UNITS: usize = 256;
const MAX_LOCAL_MESSAGE_UTF16_UNITS: usize = 384;

// The native epoch is not a renderer credential. It prevents a queued message
// from an older main-document realm being accepted after a real navigation.
#[cfg(feature = "local-server")]
#[derive(Default)]
struct HostDocumentState {
    epoch: u64,
    committed: bool,
    nonce: Option<[u8; 16]>,
    closed: bool,
}

#[cfg(feature = "local-server")]
impl HostDocumentState {
    fn started(&mut self) -> Option<u64> {
        if self.closed {
            return None;
        }
        let Some(epoch) = self.epoch.checked_add(1) else {
            self.close();
            return None;
        };
        self.epoch = epoch;
        self.invalidate();
        Some(epoch)
    }

    fn committed(&mut self, epoch: u64) -> bool {
        if self.closed || self.epoch == 0 || self.epoch != epoch || self.committed {
            return false;
        }
        self.committed = true;
        true
    }

    fn admit(&mut self, epoch: u64, nonce: [u8; 16]) -> bool {
        if self.closed || !self.committed || self.epoch != epoch || self.nonce.is_some() {
            return false;
        }
        self.nonce = Some(nonce);
        true
    }

    fn accepts(&self, nonce: &[u8; 16]) -> bool {
        !self.closed && self.committed && self.nonce.as_ref() == Some(nonce)
    }

    fn invalidate(&mut self) {
        self.committed = false;
        self.nonce = None;
    }

    fn close(&mut self) {
        self.closed = true;
        self.invalidate();
    }
}

#[cfg(feature = "local-server")]
pub(super) struct HostDocumentGate {
    state: RefCell<HostDocumentState>,
    view: RefCell<Weak<WKWebView>>,
    origin: crate::LoopbackOrigin,
    lifetime: crate::HostLifetime,
}

#[cfg(feature = "local-server")]
impl HostDocumentGate {
    fn new(origin: crate::LoopbackOrigin, lifetime: crate::HostLifetime) -> Rc<Self> {
        Rc::new(Self {
            state: RefCell::new(HostDocumentState::default()),
            view: RefCell::new(Weak::default()),
            origin,
            lifetime,
        })
    }

    pub(super) fn attach(&self, view: &WKWebView) {
        *self.view.borrow_mut() = Weak::new(view);
    }

    pub(super) fn started(&self) {
        self.state.borrow_mut().started();
    }

    pub(super) fn failed(&self) {
        self.state.borrow_mut().invalidate();
    }

    pub(super) fn close(&self) {
        self.state.borrow_mut().close();
    }

    fn current(&self, nonce: &[u8; 16]) -> bool {
        self.lifetime.is_active() && self.state.borrow().accepts(nonce)
    }

    fn trusted_view(&self, view: &WKWebView) -> bool {
        let attached = self
            .view
            .borrow()
            .load()
            .is_some_and(|attached| std::ptr::eq(&*attached, view));
        if !self.lifetime.is_active() || !attached || view.window().is_none() {
            return false;
        }
        // SAFETY: WKWebView.URL is read on the owning AppKit main thread.
        (unsafe { view.URL() }).is_some_and(|url| {
            url.user().is_none()
                && url.password().is_none()
                && url.absoluteString().is_some_and(|value| {
                    self.lifetime
                        .allows_navigation(&self.origin, &value.to_string())
                })
        })
    }

    pub(super) fn accepts(&self, view: &WKWebView, nonce: &[u8; 16]) -> bool {
        self.trusted_view(view) && self.current(nonce)
    }

    pub(super) fn committed(self: &Rc<Self>, view: &WKWebView) {
        if !self.trusted_view(view) {
            return;
        }
        let epoch = {
            let mut state = self.state.borrow_mut();
            let epoch = state.epoch;
            if !state.committed(epoch) {
                return;
            }
            epoch
        };
        let weak = Rc::downgrade(self);
        let callback = RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
            let Some(gate) = weak.upgrade() else { return };
            let Some(view) = gate.view.borrow().load() else {
                return;
            };
            if !error.is_null() || !gate.trusted_view(&view) {
                return;
            }
            // SAFETY: WebKit owns the result throughout this main-thread callback.
            let nonce = unsafe { value.as_ref() }
                .and_then(|value| value.downcast_ref::<NSString>())
                .and_then(|value| bounded_message(value, 32))
                .and_then(|value| parse_nonce(&value));
            if let Some(nonce) = nonce {
                gate.state.borrow_mut().admit(epoch, nonce);
            }
        });
        // SAFETY: The native commit and this main-frame evaluation belong to
        // the exact registered WKWebView; a later navigation fails the epoch
        // check before the asynchronous result can be admitted.
        unsafe {
            view.evaluateJavaScript_completionHandler(
                ns_string!("window===window.top?window.webuiHostPostMessage?.documentNonce:null"),
                Some(&callback),
            );
        }
    }
}

#[cfg(feature = "local-server")]
fn parse_nonce(text: &str) -> Option<[u8; 16]> {
    fn digit(value: u8) -> Option<u8> {
        match value {
            b'0'..=b'9' => Some(value - b'0'),
            b'a'..=b'f' => Some(value - b'a' + 10),
            _ => None,
        }
    }
    if text.len() != 32 {
        return None;
    }
    let mut nonce = [0_u8; 16];
    for (pair, byte) in text.as_bytes().as_chunks::<2>().0.iter().zip(&mut nonce) {
        *byte = digit(pair[0])? * 16 + digit(pair[1])?;
    }
    Some(nonce)
}

#[cfg(feature = "local-server")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalHostMessage {
    nonce: String,
    command: String,
}

#[derive(Default)]
pub(super) struct HostMessageHandlerIvars {
    #[cfg(feature = "local-server")]
    local: Option<(
        crate::LoopbackOrigin,
        crate::HostLifetime,
        Rc<HostDocumentGate>,
    )>,
}

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
            let Some((window, view)) = trusted_host_window(message, self.ivars()) else {
                return;
            };
            #[cfg(not(feature = "local-server"))]
            let _ = &view;
            let body = message.body();
            #[cfg(feature = "local-server")]
            if let Some((_, _, gate)) = &self.ivars().local {
                if let Some((nonce, command)) = decode_local_host_message(&body) {
                    if gate.accepts(&view, &nonce) {
                        run_host_message(command, &window, self.mtm());
                    }
                }
                return;
            }
            if let Some(command) = decode_host_message(&body) {
                run_host_message(command, &window, self.mtm());
            }
        }
    }
);

fn trusted_host_window(
    message: &WKScriptMessage,
    ivars: &HostMessageHandlerIvars,
) -> Option<(Retained<NSWindow>, Retained<objc2_web_kit::WKWebView>)> {
    // SAFETY: These are WebKit's native sender and current-document identities,
    // not values supplied in the renderer's message body.
    unsafe {
        let frame = message.frameInfo();
        if !frame.isMainFrame() {
            return None;
        }
        let view = message.webView()?;
        let url = view.URL()?;
        let origin = frame.securityOrigin();
        #[cfg(feature = "local-server")]
        let allowed = if let Some((local_origin, lifetime, _)) = &ivars.local {
            trusted_local_frame(
                true,
                FrameOrigin {
                    scheme: &origin.protocol(),
                    host: &origin.host(),
                    port: origin.port(),
                },
                &url,
                local_origin,
                lifetime,
            )
        } else {
            trusted_host_frame(
                true,
                &origin.protocol(),
                &origin.host(),
                origin.port(),
                &url,
            )
        };
        #[cfg(not(feature = "local-server"))]
        let allowed = {
            let _ = ivars;
            trusted_host_frame(
                true,
                &origin.protocol(),
                &origin.host(),
                origin.port(),
                &url,
            )
        };
        if !allowed {
            return None;
        }
        view.window().map(|window| (window, view))
    }
}

fn trusted_host_frame(
    main: bool,
    scheme: &NSString,
    host: &NSString,
    port: isize,
    current_url: &NSURL,
) -> bool {
    main && scheme.isEqualToString(ns_string!("webui"))
        && host.isEqualToString(ns_string!("app"))
        && port == 0
        && super::navigation::trusted_app_url(current_url)
}

#[cfg(feature = "local-server")]
struct FrameOrigin<'a> {
    scheme: &'a NSString,
    host: &'a NSString,
    port: isize,
}

#[cfg(feature = "local-server")]
fn trusted_local_frame(
    main: bool,
    frame: FrameOrigin<'_>,
    current_url: &NSURL,
    origin: &crate::LoopbackOrigin,
    lifetime: &crate::HostLifetime,
) -> bool {
    main && frame.scheme.length() == 4
        && frame.host.length() > 0
        && frame.host.length() <= 45
        && origin.matches_security_origin(
            &frame.scheme.to_string(),
            &frame.host.to_string(),
            frame.port,
        )
        && current_url
            .absoluteString()
            .is_some_and(|url| lifetime.allows_navigation(origin, &url.to_string()))
}

fn decode_host_message(body: &AnyObject) -> Option<DesktopHostMessage> {
    let text = body.downcast_ref::<NSString>()?;
    DesktopHostMessage::from_json(&bounded_message(text, MAX_MESSAGE_UTF16_UNITS)?).ok()
}

fn bounded_message(text: &NSString, max_units: usize) -> Option<String> {
    let length = text.length();
    if length == 0 || length > max_units {
        return None;
    }
    // NSString can contain unpaired surrogates. Copy bounded code units through
    // Foundation's safe getter, then validate without its infallible UTF-8 path.
    let mut units = [0_u16; MAX_LOCAL_MESSAGE_UTF16_UNITS];
    for (index, unit) in units[..length].iter_mut().enumerate() {
        *unit = text.characterAtIndex(index);
    }
    String::from_utf16(&units[..length]).ok()
}

#[cfg(feature = "local-server")]
fn decode_local_host_message(body: &AnyObject) -> Option<([u8; 16], DesktopHostMessage)> {
    let text = body.downcast_ref::<NSString>()?;
    let json = bounded_message(text, MAX_LOCAL_MESSAGE_UTF16_UNITS)?;
    if json.len() > 512 {
        return None;
    }
    let payload: LocalHostMessage = serde_json::from_str(&json).ok()?;
    let nonce = parse_nonce(&payload.nonce)?;
    let command = DesktopHostMessage::from_json(&payload.command).ok()?;
    Some((nonce, command))
}

fn run_host_message(command: DesktopHostMessage, window: &NSWindow, mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
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
        Self::from_ivars(mtm, HostMessageHandlerIvars::default())
    }

    #[cfg(feature = "local-server")]
    pub(super) fn for_local_server(
        mtm: MainThreadMarker,
        origin: crate::LoopbackOrigin,
        lifetime: crate::HostLifetime,
    ) -> Retained<Self> {
        let gate = HostDocumentGate::new(origin.clone(), lifetime.clone());
        Self::from_ivars(
            mtm,
            HostMessageHandlerIvars {
                local: Some((origin, lifetime, gate)),
            },
        )
    }

    #[cfg(feature = "local-server")]
    pub(super) fn local_gate(&self) -> Option<Rc<HostDocumentGate>> {
        self.ivars()
            .local
            .as_ref()
            .map(|(_, _, gate)| Rc::clone(gate))
    }

    #[cfg(feature = "local-server")]
    pub(super) fn attach(&self, view: &WKWebView) {
        if let Some(gate) = self.local_gate() {
            gate.attach(view);
        }
    }

    #[cfg(feature = "local-server")]
    pub(super) fn close(&self) {
        if let Some(gate) = self.local_gate() {
            gate.close();
        }
    }

    fn from_ivars(mtm: MainThreadMarker, ivars: HostMessageHandlerIvars) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ivars);
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

    #[test]
    fn host_commands_require_the_current_app_main_frame() {
        autoreleasepool(|_| {
            let scheme = NSString::from_str("webui");
            let host = NSString::from_str("app");
            let app = objc2_foundation::NSURL::URLWithString(&NSString::from_str("webui://app/"))
                .unwrap();
            let preview =
                objc2_foundation::NSURL::URLWithString(&NSString::from_str("https://other.test/"))
                    .unwrap();
            let credentialed =
                objc2_foundation::NSURL::URLWithString(&NSString::from_str("webui://user@app/"))
                    .unwrap();
            let wrong_port =
                objc2_foundation::NSURL::URLWithString(&NSString::from_str("webui://app:80/"))
                    .unwrap();

            assert!(trusted_host_frame(true, &scheme, &host, 0, &app));
            assert!(!trusted_host_frame(false, &scheme, &host, 0, &app));
            assert!(!trusted_host_frame(true, &scheme, &host, 0, &preview));
            assert!(!trusted_host_frame(true, &scheme, &host, 0, &credentialed));
            assert!(!trusted_host_frame(true, &scheme, &host, 0, &wrong_port));
            assert!(!trusted_host_frame(true, &scheme, &host, 443, &app));
            assert!(!trusted_host_frame(
                true,
                &NSString::from_str("https"),
                &host,
                0,
                &app
            ));
            assert!(!trusted_host_frame(
                true,
                &scheme,
                &NSString::from_str("app.evil"),
                0,
                &app
            ));
        });
    }

    #[cfg(feature = "local-server")]
    #[test]
    fn local_host_controls_require_owning_main_origin_and_live_server(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (owner, lifetime) = crate::HostLifetime::new();
        let origin = crate::LoopbackOrigin::from_socket_addr("127.0.0.1:3456".parse()?)?;
        let scheme = NSString::from_str("http");
        let host = NSString::from_str("127.0.0.1");
        let own = NSURL::URLWithString(&NSString::from_str("http://127.0.0.1:3456/settings"))
            .ok_or("own URL")?;
        let preview = NSURL::URLWithString(&NSString::from_str(
            "http://p-lease.preview.localhost:3456/",
        ))
        .ok_or("preview URL")?;
        let credentialed = NSURL::URLWithString(&NSString::from_str("http://user@127.0.0.1:3456/"))
            .ok_or("credentialed URL")?;
        let frame = || FrameOrigin {
            scheme: &scheme,
            host: &host,
            port: 3456,
        };
        assert!(trusted_local_frame(true, frame(), &own, &origin, &lifetime));
        assert!(!trusted_local_frame(
            false,
            frame(),
            &own,
            &origin,
            &lifetime
        ));
        assert!(!trusted_local_frame(
            true,
            frame(),
            &preview,
            &origin,
            &lifetime
        ));
        assert!(!trusted_local_frame(
            true,
            frame(),
            &credentialed,
            &origin,
            &lifetime
        ));
        assert!(!trusted_local_frame(
            true,
            FrameOrigin {
                scheme: &scheme,
                host: &NSString::from_str("127.0.0.2"),
                port: 3456,
            },
            &own,
            &origin,
            &lifetime
        ));
        assert!(!trusted_local_frame(
            true,
            FrameOrigin {
                scheme: &scheme,
                host: &host,
                port: 3457,
            },
            &own,
            &origin,
            &lifetime
        ));
        owner.revoke()?;
        assert!(!trusted_local_frame(
            true,
            frame(),
            &own,
            &origin,
            &lifetime
        ));
        Ok(())
    }

    #[cfg(feature = "local-server")]
    #[test]
    fn same_origin_stale_command_cannot_cross_navigation_or_reload() {
        let (_owner, lifetime) = crate::HostLifetime::new();
        let origin =
            crate::LoopbackOrigin::from_socket_addr("127.0.0.1:3456".parse().unwrap()).unwrap();
        let scheme = NSString::from_str("http");
        let host = NSString::from_str("127.0.0.1");
        let current = NSURL::URLWithString(&NSString::from_str("http://127.0.0.1:3456/b")).unwrap();
        // The old URL-only rule authorizes the /a command against /b.
        assert!(trusted_local_frame(
            true,
            FrameOrigin {
                scheme: &scheme,
                host: &host,
                port: 3456,
            },
            &current,
            &origin,
            &lifetime,
        ));
        let mut gate = HostDocumentState::default();
        let first = gate.started().unwrap();
        assert!(gate.committed(first));
        assert!(gate.admit(first, [1; 16]));
        assert!(gate.accepts(&[1; 16]));

        let second = gate.started().unwrap();
        assert!(gate.committed(second));
        assert!(gate.admit(second, [2; 16]));
        assert!(!gate.accepts(&[1; 16]));
        assert!(gate.accepts(&[2; 16]));

        let reload = gate.started().unwrap();
        assert!(!gate.admit(second, [3; 16]));
        assert!(gate.committed(reload));
        assert!(gate.admit(reload, [3; 16]));
        assert!(!gate.accepts(&[2; 16]));
        gate.close();
        assert!(!gate.accepts(&[3; 16]));
        assert!(gate.started().is_none());
    }

    #[cfg(feature = "local-server")]
    #[test]
    fn late_probe_failed_commit_and_owner_revocation_fail_closed() {
        let (owner, lifetime) = crate::HostLifetime::new();
        let origin =
            crate::LoopbackOrigin::from_socket_addr("127.0.0.1:3456".parse().unwrap()).unwrap();
        let gate = HostDocumentGate::new(origin, lifetime);
        let first = gate.state.borrow_mut().started().unwrap();
        assert!(gate.state.borrow_mut().committed(first));
        let second = gate.state.borrow_mut().started().unwrap();
        assert!(!gate.state.borrow_mut().admit(first, [1; 16]));
        assert!(!gate.current(&[1; 16]));
        assert!(gate.state.borrow_mut().committed(second));
        assert!(gate.state.borrow_mut().admit(second, [2; 16]));
        assert!(gate.current(&[2; 16]));
        gate.failed();
        assert!(!gate.current(&[2; 16]));
        let third = gate.state.borrow_mut().started().unwrap();
        assert!(gate.state.borrow_mut().committed(third));
        assert!(gate.state.borrow_mut().admit(third, [3; 16]));
        owner.revoke().unwrap();
        assert!(!gate.current(&[3; 16]));
        gate.close();
        assert!(!gate.current(&[3; 16]));
    }

    #[cfg(feature = "local-server")]
    #[test]
    fn local_envelope_rejects_legacy_malformed_and_oversize_messages() {
        autoreleasepool(|_| {
            let valid = NSString::from_str(
                r#"{"nonce":"01010101010101010101010101010101","command":"\"close\""}"#,
            );
            assert!(matches!(
                decode_local_host_message(&valid),
                Some((nonce, DesktopHostMessage::Close)) if nonce == [1; 16]
            ));
            for payload in [
                r#""close""#,
                r#"{"nonce":"01010101010101010101010101010101","command":"close"}"#,
                r#"{"nonce":"GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG","command":"\"close\""}"#,
                r#"{"nonce":"01010101010101010101010101010101","command":"\"close\"","extra":1}"#,
                r#"{"nonce":"01010101010101010101010101010101","command":"\"unknown\""}"#,
            ] {
                assert!(decode_local_host_message(&NSString::from_str(payload)).is_none());
            }
            assert!(decode_local_host_message(&NSString::from_str(
                &"x".repeat(MAX_LOCAL_MESSAGE_UTF16_UNITS + 1)
            ))
            .is_none());
            assert!(decode_local_host_message(&native_utf16(&[b'{' as u16, 0xd800])).is_none());
        });
    }
}
