//! Node.js native addon for the WebUI framework via napi-rs.
//!
//! Provides high-performance server-side rendering by compiling the Rust
//! WebUI handler directly into a `.node` native addon — no C ABI intermediary.
//!
//! The `render` function accepts pre-compiled protobuf protocol data (from
//! `webui build`) and streams rendered HTML fragments via a callback, enabling
//! true streaming SSR without buffering the entire response.
//!
//! ## Usage (from Node.js)
//!
//! ```js
//! import fs from 'node:fs';
//!
//! // Load the native addon
//! const mod = { exports: {} };
//! process.dlopen(mod, './target/release/libwebui_node.dylib');
//! const { render } = mod.exports;
//!
//! // Read pre-compiled protocol (from `webui build`)
//! const protocol = fs.readFileSync('./dist/protocol.bin');
//! const state = '{"name": "WebUI"}';
//!
//! // Stream rendered fragments
//! render(protocol, state, (chunk) => process.stdout.write(chunk));
//! ```

use napi::bindgen_prelude::{Buffer, Function};
use napi::Error as NapiError;
use napi_derive::napi;
use serde_json::Value;
use webui_handler::{ResponseWriter, WebUIHandler};
use webui_protocol::WebUIProtocol;

/// A writer that streams each rendered fragment to a JS callback.
struct CallbackWriter<'a, 'env> {
    callback: &'a Function<'env, String, ()>,
}

impl<'a, 'env> CallbackWriter<'a, 'env> {
    fn new(callback: &'a Function<'env, String, ()>) -> Self {
        Self { callback }
    }
}

impl ResponseWriter for CallbackWriter<'_, '_> {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        // Ignore return-value type mismatch — JS callbacks may return
        // non-undefined values (e.g. `res.write()` returns a boolean).
        let _ = self.callback.call(content.to_owned());
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

/// Render a pre-compiled WebUI protocol with JSON state, streaming each
/// fragment to the provided callback.
///
/// # Arguments
///
/// * `protocol_data` — Protobuf binary from `webui build` (zero-copy Buffer).
/// * `state_json` — JSON string with the render state.
/// * `on_chunk` — Called with each rendered HTML fragment as it is produced.
#[napi]
pub fn render(
    protocol_data: Buffer,
    state_json: String,
    on_chunk: Function<String, ()>,
) -> napi::Result<()> {
    let protocol = WebUIProtocol::from_protobuf(&protocol_data)
        .map_err(|e| NapiError::from_reason(format!("Protocol decode error: {e}")))?;

    let state: Value = serde_json::from_str(&state_json)
        .map_err(|e| NapiError::from_reason(format!("State JSON error: {e}")))?;

    let mut writer = CallbackWriter::new(&on_chunk);
    let handler = WebUIHandler::new();
    handler
        .render(&protocol, &state, &mut writer)
        .map_err(|e| NapiError::from_reason(format!("Render error: {e}")))?;

    Ok(())
}

#[cfg(test)]
mod tests;
