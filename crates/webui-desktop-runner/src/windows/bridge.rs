// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Same-origin `fetch` bridge that routes app requests through the desktop runtime.

use std::sync::Arc;

use anyhow::Result;
use serde_json::{Map, Value};
use webui_desktop::{
    DesktopHttpMethod, DesktopProtocolRequest, DesktopProtocolResponse, DesktopRuntime,
};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2, ICoreWebView2WebMessageReceivedEventArgs,
    ICoreWebView2WebMessageReceivedEventHandler,
};
use webview2_com::{
    AddScriptToExecuteOnDocumentCreatedCompletedHandler, CoTaskMemPWSTR,
    WebMessageReceivedEventHandler,
};
use windows::core::{Error as WindowsError, Result as WindowsResult};
use windows::Win32::Foundation::{E_FAIL, HWND};

use super::protocol::{read_pwstr, webui_path_from_uri};
use super::webview::handle_host_message;
use super::{FETCH_BRIDGE_KIND, FETCH_BRIDGE_RESPONSE_KIND};

const FETCH_BRIDGE_SCRIPT: &str = r#"
(() => {
  if (!window.chrome?.webview || window.__webuiDesktopFetchBridge) {
    return;
  }
  window.__webuiDesktopFetchBridge = true;
  const appOrigin = 'https://app.webui.localhost';
  const originalFetch = window.fetch.bind(window);
  let nextId = 1;
  const pending = new Map();

  const bytesToBase64 = (buffer) => {
    const bytes = new Uint8Array(buffer);
    let binary = '';
    for (let i = 0; i < bytes.length; i += 0x8000) {
      binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    }
    return btoa(binary);
  };

  const base64ToBytes = (value) => {
    const binary = atob(value || '');
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i += 1) {
      bytes[i] = binary.charCodeAt(i);
    }
    return bytes;
  };

  window.chrome.webview.addEventListener('message', (event) => {
    const message = event.data;
    if (!message || message.kind !== 'webui-desktop-fetch-response') {
      return;
    }
    const callbacks = pending.get(message.id);
    if (!callbacks) {
      return;
    }
    pending.delete(message.id);
    if (message.error) {
      callbacks.reject(new TypeError(message.error));
      return;
    }
    const emptyBodyStatus = message.status === 204 || message.status === 205 || message.status === 304;
    callbacks.resolve(new Response(emptyBodyStatus ? null : base64ToBytes(message.bodyBase64), {
      status: message.status,
      headers: message.headers || {}
    }));
  });

  window.fetch = async (input, init) => {
    const request = new Request(input, init);
    const url = new URL(request.url, location.href);
    if (url.origin !== appOrigin) {
      return originalFetch(input, init);
    }
    const id = nextId++;
    const headers = [];
    request.headers.forEach((value, key) => headers.push([key, value]));
    const hasBody = request.method !== 'GET' && request.method !== 'HEAD';
    const bodyBase64 = hasBody ? bytesToBase64(await request.clone().arrayBuffer()) : '';
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      window.chrome.webview.postMessage({
        kind: 'webui-desktop-fetch',
        id,
        method: request.method,
        url: url.href,
        headers,
        bodyBase64
      });
    });
  };
})();
"#;

/// Install the `fetch` bridge and route web messages to the runtime.
///
/// Window-control messages are handled first so they never reach the bridge.
pub(super) fn register_fetch_bridge(
    webview: &ICoreWebView2,
    runtime: Arc<DesktopRuntime>,
    hwnd: HWND,
) -> Result<ICoreWebView2WebMessageReceivedEventHandler> {
    inject_fetch_bridge_script(webview)?;
    let handler = WebMessageReceivedEventHandler::create(Box::new(move |sender, args| {
        if let (Some(webview), Some(args)) = (sender, args) {
            if handle_host_message(hwnd, &args)? {
                return Ok(());
            }
            handle_fetch_bridge_message(&webview, &runtime, &args)?;
        }
        Ok(())
    }));
    let mut token = 0_i64;
    // SAFETY: `webview` is a live COM interface and the handler is returned to
    // the caller, which keeps it alive for the lifetime of the window.
    unsafe { webview.add_WebMessageReceived(&handler, &mut token)? };
    Ok(handler)
}

/// Install the `fetch` interception script on every document.
fn inject_fetch_bridge_script(webview: &ICoreWebView2) -> Result<()> {
    let webview = webview.clone();
    let script = CoTaskMemPWSTR::from(FETCH_BRIDGE_SCRIPT);
    AddScriptToExecuteOnDocumentCreatedCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| {
            // SAFETY: `webview` is live and the script buffer outlives the call.
            unsafe {
                webview
                    .AddScriptToExecuteOnDocumentCreated(*script.as_ref().as_pcwstr(), &handler)
                    .map_err(webview2_com::Error::WindowsError)
            }
        }),
        Box::new(|result, _script_id| {
            result?;
            Ok(())
        }),
    )?;
    Ok(())
}

/// Serve one bridged `fetch` request from the desktop runtime.
fn handle_fetch_bridge_message(
    webview: &ICoreWebView2,
    runtime: &DesktopRuntime,
    args: &ICoreWebView2WebMessageReceivedEventArgs,
) -> WindowsResult<()> {
    // SAFETY: WebView2 passes a live args interface for the callback's
    // duration; the JSON is copied out before the callback returns.
    let raw = read_pwstr(|out| unsafe { args.WebMessageAsJson(out) })?;
    let Ok(message) = serde_json::from_str::<Value>(&raw) else {
        return Ok(());
    };
    if message.get("kind").and_then(Value::as_str) != Some(FETCH_BRIDGE_KIND) {
        return Ok(());
    }

    let id = message
        .get("id")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let response = match fetch_bridge_response(runtime, &message) {
        Ok(response) => response,
        Err(error) => fetch_bridge_error_response(id, error),
    };
    post_web_message_json(webview, &response)
}

/// Translate a bridged request into a runtime response payload.
fn fetch_bridge_response(
    runtime: &DesktopRuntime,
    message: &Value,
) -> std::result::Result<Value, String> {
    let id = message
        .get("id")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET");
    let url = message
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| "desktop fetch bridge request is missing a URL".to_string())?;
    let body = message
        .get("bodyBase64")
        .and_then(Value::as_str)
        .map(decode_base64)
        .transpose()?
        .unwrap_or_default();
    let accept = header_value(message.get("headers"), "accept");
    let path = webui_path_from_uri(url);
    let request = DesktopProtocolRequest {
        method: DesktopHttpMethod::parse(method),
        path: &path,
        body: &body,
        wants_json: accept.contains("json") || accept.contains("ndjson"),
    };
    let response = runtime
        .handle_request(&request)
        .map_err(|error| error.chain_message())?;
    Ok(fetch_bridge_success_response(id, response))
}

/// Build the error payload returned to a rejected bridged request.
fn fetch_bridge_error_response(id: u64, error: String) -> Value {
    let mut map = Map::new();
    map.insert(
        "kind".to_string(),
        Value::String(FETCH_BRIDGE_RESPONSE_KIND.to_string()),
    );
    map.insert("id".to_string(), Value::from(id));
    map.insert("error".to_string(), Value::String(error));
    Value::Object(map)
}

/// Build the success payload for a completed bridged request.
fn fetch_bridge_success_response(id: u64, response: DesktopProtocolResponse) -> Value {
    let mut headers = Map::new();
    headers.insert(
        "content-type".to_string(),
        Value::String(response.content_type),
    );

    let mut map = Map::new();
    map.insert(
        "kind".to_string(),
        Value::String(FETCH_BRIDGE_RESPONSE_KIND.to_string()),
    );
    map.insert("id".to_string(), Value::from(id));
    map.insert(
        "status".to_string(),
        Value::from(u64::from(response.status)),
    );
    map.insert("headers".to_string(), Value::Object(headers));
    map.insert(
        "bodyBase64".to_string(),
        Value::String(encode_base64(&response.body)),
    );
    Value::Object(map)
}

/// Look up a header value from the bridged header pairs.
fn header_value(headers: Option<&Value>, name: &str) -> String {
    let Some(headers) = headers.and_then(Value::as_array) else {
        return String::new();
    };
    for pair in headers {
        let Some(pair) = pair.as_array() else {
            continue;
        };
        if pair.len() != 2 {
            continue;
        }
        let key = pair[0].as_str().unwrap_or_default();
        if key.eq_ignore_ascii_case(name) {
            return pair[1].as_str().unwrap_or_default().to_string();
        }
    }
    String::new()
}

/// Post a JSON reply back to web content.
fn post_web_message_json(webview: &ICoreWebView2, value: &Value) -> WindowsResult<()> {
    let text = serde_json::to_string(value).map_err(|_| WindowsError::from(E_FAIL))?;
    let text = CoTaskMemPWSTR::from(text.as_str());
    // SAFETY: `webview` is a live COM interface and the text buffer outlives
    // this call.
    unsafe { webview.PostWebMessageAsJson(*text.as_ref().as_pcwstr()) }
}

/// Encode bytes as standard base64 with padding.
fn encode_base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let (chunks, rem) = bytes.as_chunks::<3>();
    for chunk in chunks {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
        out.push(base64_char((n >> 18) & 0x3f));
        out.push(base64_char((n >> 12) & 0x3f));
        out.push(base64_char((n >> 6) & 0x3f));
        out.push(base64_char(n & 0x3f));
    }
    if rem.len() == 1 {
        let n = u32::from(rem[0]) << 16;
        out.push(base64_char((n >> 18) & 0x3f));
        out.push(base64_char((n >> 12) & 0x3f));
        out.push('=');
        out.push('=');
    } else if rem.len() == 2 {
        let n = (u32::from(rem[0]) << 16) | (u32::from(rem[1]) << 8);
        out.push(base64_char((n >> 18) & 0x3f));
        out.push(base64_char((n >> 12) & 0x3f));
        out.push(base64_char((n >> 6) & 0x3f));
        out.push('=');
    }
    out
}

fn base64_char(index: u32) -> char {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    usize::try_from(index)
        .ok()
        .and_then(|index| TABLE.get(index))
        .map_or('A', |byte| char::from(*byte))
}

/// Decode standard padded base64 into bytes.
fn decode_base64(input: &str) -> std::result::Result<Vec<u8>, String> {
    let bytes = input.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return Err("desktop fetch bridge body is not valid base64".to_string());
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.as_chunks::<4>().0 {
        decode_base64_group(chunk, &mut out)?;
    }
    Ok(out)
}

/// Decode one four-character base64 group into up to three bytes.
fn decode_base64_group(group: &[u8; 4], out: &mut Vec<u8>) -> std::result::Result<(), String> {
    let a = base64_value(group[0])?;
    let b = base64_value(group[1])?;
    let c = base64_value_or_padding(group[2])?;
    let d = base64_value_or_padding(group[3])?;
    if a == 64 || b == 64 || (c == 64 && d != 64) {
        return Err("desktop fetch bridge body has invalid base64 padding".to_string());
    }

    let c_bits = if c == 64 { 0 } else { c };
    let d_bits = if d == 64 { 0 } else { d };
    let n =
        (u32::from(a) << 18) | (u32::from(b) << 12) | (u32::from(c_bits) << 6) | u32::from(d_bits);
    out.push(base64_decoded_byte((n >> 16) & 0xff)?);
    if c != 64 {
        out.push(base64_decoded_byte((n >> 8) & 0xff)?);
    }
    if d != 64 {
        out.push(base64_decoded_byte(n & 0xff)?);
    }
    Ok(())
}

fn base64_decoded_byte(value: u32) -> std::result::Result<u8, String> {
    u8::try_from(value).map_err(|_| "desktop fetch bridge body is too large".to_string())
}

fn base64_value_or_padding(byte: u8) -> std::result::Result<u8, String> {
    if byte == b'=' {
        Ok(64)
    } else {
        base64_value(byte)
    }
}

fn base64_value(byte: u8) -> std::result::Result<u8, String> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err("desktop fetch bridge body contains invalid base64".to_string()),
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn fetch_bridge_base64_round_trips_body_bytes() {
        let body = b"{\"name\":\"Sarah\"}\0\xff";
        let encoded = encode_base64(body);
        assert_eq!(decode_base64(&encoded).unwrap(), body);
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
    }
}
