// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Bounded control parsing and native document bookkeeping, without COM.

use crate::ipc::{DocumentActivation, Hello, IpcError, IpcErrorCode};
use serde::Deserialize;

pub(super) use super::protocol::app_url;

pub(super) const MAX_CONTROL_UNITS: usize = 4096;
pub(super) const MAX_TASKS: usize = 128;

pub(super) fn reserved_path(uri: &str) -> Option<&str> {
    if !app_url(uri) {
        return None;
    }
    let path = uri.strip_prefix(super::APP_ORIGIN)?;
    let route = path.split(['?', '#']).next()?;
    // These are immutable SDK assets, not authenticated binary endpoints.
    if matches!(route, "/_webui/ipc/bootstrap.js" | "/_webui/ipc/runtime.js") {
        return None;
    }
    (route == "/_webui/ipc" || route.starts_with("/_webui/ipc/")).then_some(path)
}

pub(super) fn decimal(value: &str) -> Option<u64> {
    if value.is_empty()
        || value.len() > 20
        || value.starts_with('0')
        || !value.bytes().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    value.parse().ok()
}

pub(super) fn nonce(value: &str) -> Option<[u8; 16]> {
    if value.len() != 32 {
        return None;
    }
    let mut out = [0; 16];
    for (byte, pair) in out.iter_mut().zip(value.as_bytes().as_chunks::<2>().0) {
        let digit = |c| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        };
        *byte = digit(pair[0])? * 16 + digit(pair[1])?;
    }
    Some(out)
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(super) enum Control {
    #[serde(rename_all = "camelCase")]
    Hello {
        call_id: String,
        navigation: String,
        document_nonce: String,
        challenge: String,
        wire_version: u32,
        contract_name: String,
        contract_major: u32,
        schema_hash: String,
    },
    Disconnect {
        generation: String,
        token: String,
    },
}

pub(super) use crate::native_ipc::NativeHello as HelloCall;

impl Control {
    pub fn into_hello(self) -> Option<HelloCall> {
        let Self::Hello {
            call_id,
            navigation,
            document_nonce,
            challenge,
            wire_version,
            contract_name,
            contract_major,
            schema_hash,
        } = self
        else {
            return None;
        };
        if call_id.is_empty()
            || call_id.len() > 32
            || contract_name.len() > 256
            || schema_hash.len() != 64
        {
            return None;
        }
        Some(HelloCall {
            call_id,
            proof: DocumentActivation {
                navigation: decimal(&navigation)?,
                document_nonce: nonce(&document_nonce)?,
                challenge: nonce(&challenge)?,
            },
            hello: Hello {
                wire_version,
                contract_name,
                contract_major,
                schema_hash,
            },
        })
    }
}

#[derive(Default)]
pub(super) struct Document {
    pub epoch: crate::document::DocumentEpoch,
    pub native_id: Option<u64>,
    pub hello_pending: bool,
    pub generation: Option<u64>,
    pub token: Option<String>,
    pub max_frame_bytes: Option<usize>,
    pub proof: Option<DocumentActivation>,
    pub unavailable_code: Option<IpcErrorCode>,
}

impl Document {
    /// NavigationStarting excludes same-document changes; redirects retain ID.
    pub fn start(&mut self, native_id: u64) -> Option<u64> {
        if self.epoch.closed || self.native_id == Some(native_id) {
            return None;
        }
        let next = self.epoch.advance()?;
        *self = Self {
            epoch: self.epoch,
            native_id: Some(native_id),
            unavailable_code: self.native_id.map(|_| IpcErrorCode::Navigated),
            ..Self::default()
        };
        Some(next)
    }

    pub fn current(&self, navigation: u64) -> bool {
        self.epoch.current(navigation)
    }

    pub fn request_error(&self) -> Option<IpcErrorCode> {
        if self.epoch.closed {
            Some(IpcErrorCode::Closed)
        } else if self.current(self.epoch.navigation) {
            None
        } else {
            Some(self.unavailable_code.unwrap_or(IpcErrorCode::NotReady))
        }
    }

    pub fn accepts(&self, proof: &DocumentActivation) -> bool {
        self.epoch.accepts(self.proof.as_ref(), proof)
    }

    pub fn begin_hello(&mut self, proof: &DocumentActivation) -> bool {
        if !self.accepts(proof) || self.hello_pending || self.generation.is_some() {
            return false;
        }
        self.hello_pending = true;
        true
    }
}

/// Keep the challenge behind the wrapper's own nonce check. A replacement
/// document's fake activate method must never receive an old proof.
pub(super) fn activation_script(proof: &DocumentActivation) -> serde_json::Result<String> {
    let activate = crate::native_ipc::activation_script(proof)?;
    Ok(format!(
        "(()=>{{'use strict';if(window!==window.top||location.origin!=='https://app.webui.localhost')return false;return {activate}===true;}})()"
    ))
}

#[cold]
#[inline(never)]
pub(super) fn error(code: IpcErrorCode) -> IpcError {
    IpcError::new(
        code,
        code.as_str(),
        "reload the trusted app document or reduce concurrent IPC work",
    )
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn disconnect_without_credential_is_not_a_control_message() {
        assert!(
            serde_json::from_str::<Control>(r#"{"kind":"disconnect","generation":"1"}"#).is_err()
        );
        let message = serde_json::from_str::<Control>(
            r#"{"kind":"disconnect","generation":"1","token":"01010101010101010101010101010101"}"#,
        )
        .unwrap();
        assert!(
            matches!(message, Control::Disconnect { generation, token } if generation == "1" && nonce(&token).is_some())
        );
    }

    #[test]
    fn duplicate_hello_cannot_replace_the_pending_call() {
        let mut document = Document::default();
        let navigation = document.start(1).unwrap();
        document.epoch.commit();
        let proof = DocumentActivation {
            navigation,
            document_nonce: [1; 16],
            challenge: [2; 16],
        };
        document.proof = Some(proof.clone());
        assert!(document.begin_hello(&proof));
        assert!(!document.begin_hello(&proof));
        assert!(document.accepts(&proof));
        let mut forged = proof.clone();
        forged.challenge = [3; 16];
        assert!(!document.begin_hello(&forged));
        assert!(!Document::default().begin_hello(&proof));
        document.start(2);
        assert!(!document.begin_hello(&proof));
    }

    #[test]
    fn evaluated_wrapper_never_hands_old_proof_to_replacement_bootstrap() {
        let proof = DocumentActivation {
            navigation: 1,
            document_nonce: [1; 16],
            challenge: [2; 16],
        };
        let script = serde_json::to_string(&activation_script(&proof).unwrap()).unwrap();
        // Execute the production wrapper in a JS engine, not a source-text
        // assertion. Node is already part of the repository's test toolchain.
        let harness = format!(
            r#"
const vm = require('node:vm');
const assert = require('node:assert/strict');
function run(nonce, main = true, origin = 'https://app.webui.localhost') {{
  let lookedUp = 0;
  const calls = [];
  const window = {{}};
  window.top = main ? window : {{}};
  window.__webuiDesktopIpcV2 = {{
    documentNonce: nonce,
    get activate() {{ lookedUp++; return proof => {{ calls.push(proof); return true; }}; }}
  }};
  const result = vm.runInNewContext({script}, {{window, location: {{origin}}}});
  return {{result, lookedUp, calls}};
}}
for (const result of [
  run('03'.repeat(16)), // Same URL, replacement document and fake bootstrap.
  run('01'.repeat(16), false),
  run('01'.repeat(16), true, 'https://remote.example')
]) {{
  assert.equal(result.result, false);
  assert.equal(result.lookedUp, 0);
  assert.equal(result.calls.length, 0);
}}
const current = run('01'.repeat(16));
assert.equal(current.result, true);
assert.equal(current.calls.length, 1);
assert.equal(current.calls[0].navigation, '1');
assert.equal(current.calls[0].challenge, '02'.repeat(16));
"#
        );
        let output = std::process::Command::new("node")
            .args(["-e", &harness])
            .output()
            .expect("Node.js from the repository test toolchain must be installed");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn authority_is_exact_and_reserved_routes_cannot_escape() {
        assert!(app_url("https://app.webui.localhost/a"));
        assert!(app_url("https://app.webui.localhost#fragment"));
        for url in [
            "https://app.webui.localhost.evil/a",
            "https://app.webui.localhost@evil/a",
            "https://app.webui.localhost:444/a",
            "https://evil/a",
            "http://app.webui.localhost/a",
        ] {
            assert!(!app_url(url));
        }
        assert_eq!(
            reserved_path("https://app.webui.localhost/_webui/ipc?token=x"),
            Some("/_webui/ipc?token=x")
        );
        assert!(reserved_path("https://app.webui.localhost/_webui/ipc-other").is_none());
        assert!(reserved_path("https://app.webui.localhost/_webui/ipc/runtime.js").is_none());
        assert!(reserved_path("https://app.webui.localhost/_webui/ipc/bootstrap.js?v=2").is_none());
    }

    #[test]
    fn late_resource_requests_preserve_native_retirement_reason() {
        let mut document = Document::default();
        assert_eq!(document.request_error(), Some(IpcErrorCode::NotReady));
        document.start(10).unwrap();
        assert_eq!(document.request_error(), Some(IpcErrorCode::NotReady));
        document.epoch.commit();
        assert_eq!(document.request_error(), None);
        assert_eq!(document.start(10), None);
        assert_eq!(document.request_error(), None);

        document.start(11).unwrap();
        // An outgoing GET/POST may arrive before its queued JS close control.
        assert_eq!(document.request_error(), Some(IpcErrorCode::Navigated));
        assert_eq!(document.start(11), None);
        assert_eq!(document.request_error(), Some(IpcErrorCode::Navigated));
        document.epoch.commit();
        assert_eq!(document.request_error(), None);

        for code in [IpcErrorCode::Closed, IpcErrorCode::Transport] {
            document.epoch.committed = false;
            document.unavailable_code = Some(code);
            assert_eq!(document.request_error(), Some(code));
        }
        document.epoch.closed = true;
        assert_eq!(document.request_error(), Some(IpcErrorCode::Closed));
    }

    #[test]
    fn old_same_url_proof_cannot_match_new_document_or_reply() {
        let mut document = Document::default();
        let old = document.start(10).unwrap();
        document.epoch.commit();
        let proof = DocumentActivation {
            navigation: old,
            document_nonce: [1; 16],
            challenge: [2; 16],
        };
        document.proof = Some(proof.clone());
        assert!(document.accepts(&proof));
        assert_eq!(document.start(10), None); // Redirect, not a new document.
        assert!(document.accepts(&proof));
        let new = document.start(11).unwrap();
        document.epoch.commit();
        document.proof = Some(DocumentActivation {
            navigation: new,
            document_nonce: [3; 16],
            challenge: [4; 16],
        });
        assert!(!document.current(old));
        assert!(!document.accepts(&proof));
        let mut forged = proof;
        forged.navigation = new;
        assert!(!document.accepts(&forged));
        document.epoch.closed = true;
        assert!(!document.current(new));
    }

    #[test]
    fn correlation_and_proof_fields_are_bounded_canonical_values() {
        assert_eq!(decimal("18446744073709551615"), Some(u64::MAX));
        for bad in ["0", "01", "-1", "1.0", "18446744073709551616"] {
            assert_eq!(decimal(bad), None);
        }
        assert_eq!(nonce("0123456789abcdef0123456789abcdef").unwrap()[0], 1);
        assert!(nonce("0123456789ABCDEF0123456789ABCDEF").is_none());
        assert!(nonce("🦀0000000000000000000000000000").is_none());
    }
}
