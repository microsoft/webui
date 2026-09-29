// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Per-document local HTTP window controls. This is not application IPC.
//! Only CoreWebView2's main-frame WebMessageReceived event is registered;
//! subframe WebMessageReceived is deliberately never subscribed.

use std::cell::RefCell;
use std::fmt::Write;
use std::rc::Rc;

use serde::Deserialize;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2, ICoreWebView2WebMessageReceivedEventArgs,
};
use webview2_com::{CoTaskMemPWSTR, ExecuteScriptCompletedHandler};
use windows::core::Result as WindowsResult;

use super::command::execute_host_message;
use super::protocol::read_pwstr_bounded;
use crate::{DesktopHostMessage, HostLifetime, LoopbackOrigin};

const MAX_SOURCE_UNITS: usize = 2048;
const MAX_MESSAGE_UNITS: usize = 768;

struct Proof {
    token: [u8; 16],
    source: String,
}

#[derive(Default)]
struct State {
    navigation_id: Option<u64>,
    proof: Option<Proof>,
    closed: bool,
}

pub(super) struct LocalControls {
    origin: LoopbackOrigin,
    lifetime: HostLifetime,
    state: RefCell<State>,
}

impl LocalControls {
    pub(super) fn new(origin: LoopbackOrigin, lifetime: HostLifetime) -> Rc<Self> {
        Rc::new(Self {
            origin,
            lifetime,
            state: RefCell::new(State::default()),
        })
    }

    /// Called only for allowed top-level navigations on the owning STA.
    pub(super) fn start(&self, id: u64) {
        let mut state = self.state.borrow_mut();
        state.proof = None;
        state.navigation_id = (id != 0 && !state.closed && self.lifetime.is_active()).then_some(id);
    }

    pub(super) fn close(&self) {
        let mut state = self.state.borrow_mut();
        state.closed = true;
        state.navigation_id = None;
        state.proof = None;
    }

    /// Grant only the successfully completed *matching* main document. The
    /// random token rules out queued messages from a retired same-URL document.
    pub(super) fn completed(self: &Rc<Self>, webview: &ICoreWebView2, id: u64) {
        if id == 0 || !self.active_navigation(id) {
            return;
        }
        // SAFETY: This WebView2 belongs to the STA running its completion.
        let Ok(source) = read_pwstr_bounded(MAX_SOURCE_UNITS, |out| unsafe { webview.Source(out) })
        else {
            return;
        };
        if !self.lifetime.allows_navigation(&self.origin, &source) {
            return;
        }
        let mut token = [0_u8; 16];
        if let Err(error) = getrandom::fill(&mut token) {
            eprintln!(
                "WebUI: cannot grant local window controls without a document nonce: {error}"
            );
            return;
        }
        let Some(script) = drag_script(token) else {
            eprintln!("WebUI: local drag-region helper is unavailable; controls remain disabled");
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            if state.closed || state.navigation_id != Some(id) || !self.lifetime.is_active() {
                return;
            }
            state.proof = Some(Proof { token, source });
        }
        let weak = Rc::downgrade(self);
        let completion = ExecuteScriptCompletedHandler::create(Box::new(move |result, value| {
            // A policy-blocked or failed script can return a successful COM
            // call with JSON `null`. Never retain authority without the
            // current main document's explicit installation acknowledgement.
            if !installation_acknowledged(result.is_ok(), &value) {
                if let Some(controls) = weak.upgrade() {
                    controls.revoke_if(token);
                }
                eprintln!("WebUI: failed to install local window controls in the current document");
            }
            Ok(())
        }));
        let script = CoTaskMemPWSTR::from(script.as_str());
        // SAFETY: ExecuteScript targets the current main frame on this STA;
        // the immutable script is copied during the call. If navigation races,
        // the old nonce cannot authorize messages from the next document.
        if let Err(error) =
            unsafe { webview.ExecuteScript(*script.as_ref().as_pcwstr(), &completion) }
        {
            self.revoke_if(token);
            eprintln!("WebUI: failed to schedule local window controls: {error}");
        }
    }

    fn active_navigation(&self, id: u64) -> bool {
        let state = self.state.borrow();
        !state.closed && state.navigation_id == Some(id) && self.lifetime.is_active()
    }

    fn revoke_if(&self, token: [u8; 16]) {
        let mut state = self.state.borrow_mut();
        if state
            .proof
            .as_ref()
            .is_some_and(|proof| proof.token == token)
        {
            state.proof = None;
        }
    }

    pub(super) fn message(
        &self,
        webview: &ICoreWebView2,
        hwnd: windows::Win32::Foundation::HWND,
        args: &ICoreWebView2WebMessageReceivedEventArgs,
    ) -> WindowsResult<()> {
        // The native event is main-frame-only. Both URLs come from WebView2,
        // never the posted JSON. Require the exact current finished document,
        // not merely the same loopback origin.
        let sender = read_pwstr_bounded(MAX_SOURCE_UNITS, |out| unsafe { args.Source(out) })?;
        let current = read_pwstr_bounded(MAX_SOURCE_UNITS, |out| unsafe { webview.Source(out) })?;
        if !self.trusted_source(&sender, &current) {
            return Ok(());
        }
        let raw = read_pwstr_bounded(MAX_MESSAGE_UNITS, |out| unsafe {
            args.WebMessageAsJson(out)
        })?;
        if let Some(command) = self.decode(&sender, &current, &raw) {
            execute_host_message(hwnd, command);
        }
        Ok(())
    }

    fn trusted_source(&self, sender: &str, current: &str) -> bool {
        let state = self.state.borrow();
        !state.closed
            && self.lifetime.allows_navigation(&self.origin, sender)
            && sender == current
            && state
                .proof
                .as_ref()
                // Same-document history changes may update Source without
                // creating another document. The per-document nonce still
                // binds this event to the finished main frame.
                .is_some_and(|proof| self.lifetime.allows_navigation(&self.origin, &proof.source))
    }

    fn decode(&self, sender: &str, current: &str, raw: &str) -> Option<DesktopHostMessage> {
        if raw.len() > MAX_MESSAGE_UNITS || !self.trusted_source(sender, current) {
            return None;
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Control<'a> {
            #[serde(rename = "webuiHostControl")]
            token: &'a str,
            command: &'a str,
        }
        let control: Control<'_> = serde_json::from_str(raw).ok()?;
        let state = self.state.borrow();
        let proof = state.proof.as_ref()?;
        if !token_matches(&proof.token, control.token) {
            return None;
        }
        DesktopHostMessage::from_json(control.command).ok()
    }
}

fn token_matches(token: &[u8; 16], text: &str) -> bool {
    if text.len() != 32 {
        return false;
    }
    let mut mismatch = 0_u8;
    for (digits, byte) in text.as_bytes().as_chunks::<2>().0.iter().zip(token) {
        let hi = char::from(digits[0]).to_digit(16);
        let lo = char::from(digits[1]).to_digit(16);
        mismatch |= u8::from(hi.is_none() || lo.is_none());
        let value = (hi.unwrap_or(0) << 4) | lo.unwrap_or(0);
        mismatch |= u8::try_from(value).unwrap_or(0) ^ byte;
    }
    mismatch == 0
}

fn installation_acknowledged(native_ok: bool, result_json: &str) -> bool {
    native_ok && result_json == "true"
}

/// Reuse the shared drag/no-drag event algorithm, but keep the nonce inside
/// its event-listener closure. No parent-visible host global is installed.
/// Native ExecuteScript does not insert an inline DOM <script>/<style> node;
/// the HTTP host owns nonce attributes on any trusted CSS it serves.
fn drag_script(token: [u8; 16]) -> Option<String> {
    const POST: &str =
        "const p=m=>window.webuiHostPostMessage&&window.webuiHostPostMessage(JSON.stringify(m));";
    const DRAG: &str = "const d=e=>{";
    const MOVE: &str = "document.addEventListener('pointermove',e=>{";
    let shared = crate::DRAG_REGION_SCRIPT;
    if !shared.contains(POST) || !shared.contains(DRAG) || !shared.contains(MOVE) {
        return None;
    }
    let mut token_hex = String::with_capacity(32);
    for byte in token {
        let _ = write!(token_hex, "{byte:02x}");
    }
    let post = format!(
        "if(window!==window.top||!window.chrome?.webview)return false;\
         const token='{token_hex}';\
         const p=m=>window.chrome.webview.postMessage(\
         {{webuiHostControl:token,command:JSON.stringify(m)}});"
    );
    let script = shared
        .replacen(POST, &post, 1)
        .replacen(DRAG, "const d=e=>{if(!e.isTrusted)return false;", 1)
        .replacen(
            MOVE,
            "document.addEventListener('pointermove',e=>{if(!e.isTrusted)return;",
            1,
        );
    // The shared IIFE normally returns undefined. Its final listener call
    // needs a statement separator before the acknowledgement. Match only the
    // terminal closure; never splice into a nested expression.
    let prefix = script.strip_suffix("})();")?;
    Some(format!("{prefix};return true;}})();"))
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    fn controls() -> (Rc<LocalControls>, crate::HostLifetimeOwner) {
        let (owner, lifetime) = HostLifetime::new();
        let origin = LoopbackOrigin::from_socket_addr("127.0.0.1:4312".parse().unwrap()).unwrap();
        (LocalControls::new(origin, lifetime), owner)
    }

    #[test]
    fn exact_document_origin_token_and_revocation_gate_window_commands() {
        let (controls, owner) = controls();
        let source = "http://127.0.0.1:4312/";
        controls.start(1);
        assert!(!controls.trusted_source(source, source));
        controls.state.borrow_mut().proof = Some(Proof {
            token: [0x3a; 16],
            source: source.into(),
        });
        let raw = r#"{"webuiHostControl":"3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a","command":"\"toggle-maximize\""}"#;
        assert_eq!(
            controls.decode(source, source, raw),
            Some(DesktopHostMessage::ToggleMaximize)
        );
        let same_document_route = "http://127.0.0.1:4312/next";
        assert_eq!(
            controls.decode(same_document_route, same_document_route, raw),
            Some(DesktopHostMessage::ToggleMaximize)
        );
        assert_eq!(
            controls.decode(source, "http://127.0.0.1:4312/next", raw),
            None
        );
        assert_eq!(controls.decode("http://127.0.0.1:4313/", source, raw), None);
        assert_eq!(
            controls.decode(source, source, &raw.replace("3a3a", "3b3a")),
            None
        );
        controls.start(2);
        assert_eq!(controls.decode(source, source, raw), None);
        owner.revoke().unwrap();
        assert_eq!(controls.decode(source, source, raw), None);
    }

    #[test]
    fn private_drag_helper_posts_an_object_and_requires_real_main_frame_input() {
        let script = drag_script([0x3a; 16]).unwrap();
        assert!(script.contains("window!==window.top"));
        assert!(!script.contains("window.webuiHostPostMessage"));
        assert!(script.contains("postMessage({webuiHostControl:token,command:JSON.stringify(m)})"));
        assert!(!script.contains("postMessage(JSON.stringify("));
        assert!(script.contains("if(!e.isTrusted)return false"));
        assert!(script.contains("if(!e.isTrusted)return;"));
        assert!(script.ends_with("});return true;})();"));
        assert!(script.contains("3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a"));
        assert!(crate::DRAG_REGION_SCRIPT.contains("pointermove"));
        assert!(crate::DRAG_REGION_SCRIPT.contains("dblclick"));
        assert!(crate::DRAG_REGION_SCRIPT.contains("toggle-maximize"));
        let (controls, _owner) = controls();
        assert!(!controls.active_navigation(1));
        controls.start(1);
        assert!(controls.active_navigation(1));
        assert!(!controls.active_navigation(2));
        controls.close();
        assert!(!controls.active_navigation(1));
    }

    #[test]
    fn webview_message_as_json_object_decodes_but_json_string_does_not() {
        let (controls, _owner) = controls();
        let source = "http://127.0.0.1:4312/";
        controls.start(5);
        controls.state.borrow_mut().proof = Some(Proof {
            token: [0x3a; 16],
            source: source.into(),
        });
        // WebView2 WebMessageAsJson reports this object directly when JS
        // calls postMessage({..}), not as a JSON string of serialized JSON.
        let object = serde_json::to_string(&serde_json::json!({
            "webuiHostControl": "3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a",
            "command": "\"start-drag\""
        }))
        .unwrap();
        assert_eq!(
            controls.decode(source, source, &object),
            Some(DesktopHostMessage::StartDrag)
        );
        let incorrectly_wrapped = serde_json::to_string(&object).unwrap();
        assert_eq!(controls.decode(source, source, &incorrectly_wrapped), None);
    }

    #[test]
    fn policy_block_or_missing_main_frame_ack_cannot_admit_controls() {
        assert!(installation_acknowledged(true, "true"));
        assert!(!installation_acknowledged(true, "null"));
        assert!(!installation_acknowledged(true, "false"));
        assert!(!installation_acknowledged(false, "true"));
    }
}
