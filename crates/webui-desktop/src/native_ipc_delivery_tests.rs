// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;
use crate::ipc::{DocumentActivation, NativeControl};

#[test]
fn retirement_script_reaches_only_its_outgoing_document_without_credentials() {
    let proof = DocumentActivation {
        navigation: 1,
        document_nonce: [1; 16],
        challenge: [2; 16],
    };
    for code in [
        IpcErrorCode::Navigated,
        IpcErrorCode::Closed,
        IpcErrorCode::Transport,
    ] {
        let script = control_script(
            &proof,
            NativeControl::Closed {
                generation: 7,
                code,
            },
        )
        .unwrap();
        assert!(!script.contains("challenge"));
        assert!(!script.contains("token"));
        let script = serde_json::to_string(&script).unwrap();
        let code = serde_json::to_string(code.as_str()).unwrap();
        let harness = format!(
            r#"
const vm = require('node:vm');
const assert = require('node:assert/strict');
for (const nonce of ['01'.repeat(16), '03'.repeat(16), undefined]) {{
  const messages = [];
  let receiverLookups = 0;
  const window = {{
    __webuiDesktopIpcV2: nonce ? {{ documentNonce: nonce }} : undefined,
    get __webuiDesktopIpcReceiveV2() {{
      receiverLookups++;
      return message => messages.push(JSON.parse(JSON.stringify(message)));
    }}
  }};
  vm.runInNewContext({script}, {{ window }});
  if (nonce === '01'.repeat(16)) {{
    assert.deepEqual(messages, [{{ kind: 'closed', generation: '7', code: {code} }}]);
    assert.equal(receiverLookups, 1);
  }} else {{
    assert.equal(messages.length, 0);
    assert.equal(receiverLookups, 0);
  }}
}}
"#
        );
        let result = std::process::Command::new("node")
            .args(["-e", &harness])
            .output()
            .expect("Node.js from the repository test toolchain must be installed");
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
