// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// FFI crate requires unsafe for C-compatible ABI boundary.
#![allow(unsafe_code)]

//! WebUI FFI (Foreign Function Interface) for interoperability with other languages.
//!
//! This crate provides C-compatible APIs for the WebUI handler to be used from languages
//! like Go, C#, Python, etc.
//!
//! ## Quick Start
//!
//! The simplest way to render an HTML template with data:
//!
//! ```c
//! char *result = webui_render("<h1>{{title}}</h1>", "{\"title\":\"Hello\"}");
//! if (result == NULL) {
//!     const char *err = webui_last_error();
//!     // handle error...
//! } else {
//!     // use result...
//!     webui_free(result);
//! }
//! ```
//!
//! ## Error Handling
//!
//! All functions that can fail return `NULL` on error. Call [`webui_last_error`] to
//! retrieve a human-readable error message. The error string is valid until the next
//! FFI call on the same thread (follows the POSIX `dlerror()` pattern).

use serde_json::Value;
use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use webui_handler::plugin::fast_v2::FastV2HydrationPlugin;
use webui_handler::plugin::fast_v3::FastV3HydrationPlugin;
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::{RenderOptions, ResponseWriter, WebUIHandler};
#[cfg(feature = "parser")]
use webui_parser::HtmlParser;
use webui_protocol::WebUIProtocol;

// ---------------------------------------------------------------------------
// Thread-local error storage (POSIX dlerror() pattern)
// ---------------------------------------------------------------------------

thread_local! {
    /// Stores the last error message for the current thread.
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

/// Record an error message so that `webui_last_error()` can return it.
fn set_last_error(msg: impl Into<String>) {
    let mut bytes = msg.into().into_bytes();
    if let Some(nul_pos) = bytes.iter().position(|byte| *byte == 0) {
        bytes.truncate(nul_pos);
    }

    // SAFETY: Any interior NUL byte was removed by truncating at its first position.
    let c_string = unsafe { CString::from_vec_unchecked(bytes) };
    LAST_ERROR.with(|cell| {
        cell.replace(Some(c_string));
    });
}

/// Clear any previously stored error.
fn clear_last_error() {
    LAST_ERROR.with(|cell| {
        cell.replace(None);
    });
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Opaque context wrapping a `WebUIHandler`.
struct HandlerContext {
    handler: WebUIHandler,
    /// CSP nonce for inline `<script>` tags (set via `webui_handler_set_nonce`).
    nonce: Option<String>,
}

/// A simple string buffer for collecting rendered output.
struct StringResponseWriter {
    content: String,
}

impl StringResponseWriter {
    fn new() -> Self {
        Self {
            content: String::new(),
        }
    }
}

impl ResponseWriter for StringResponseWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.content.push_str(content);
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// FFI: error reporting
// ---------------------------------------------------------------------------

/// Return the last error message, or `NULL` if no error has occurred.
///
/// The returned pointer is valid until the next FFI call **on the same thread**.
/// Callers **must not** free the returned pointer.
///
/// # Thread Safety
///
/// Each thread has its own independent error state.
#[no_mangle]
pub extern "C" fn webui_last_error() -> *const c_char {
    LAST_ERROR.with(|cell| {
        let borrow = cell.borrow();
        match borrow.as_ref() {
            Some(c_str) => c_str.as_ptr(),
            None => std::ptr::null(),
        }
    })
}

// ---------------------------------------------------------------------------
// FFI: handler lifecycle
// ---------------------------------------------------------------------------

/// Create a new WebUI handler instance.
///
/// Returns an opaque pointer that must be passed to other `webui_handler_*`
/// functions and eventually freed with [`webui_handler_destroy`].
#[no_mangle]
pub extern "C" fn webui_handler_create() -> *mut c_void {
    let handler = WebUIHandler::new();
    let context = Box::new(HandlerContext {
        handler,
        nonce: None,
    });
    Box::into_raw(context) as *mut c_void
}

/// Create a new WebUI handler instance with a named plugin.
///
/// # Arguments
///
/// * `plugin_id` - Null-terminated UTF-8 string identifying the plugin.
///   Refer to the CLI/crate documentation for the current list of supported
///   identifiers.
///
/// # Returns
///
/// An opaque pointer that must be freed with [`webui_handler_destroy`],
/// or `NULL` on error (call [`webui_last_error`] for details).
///
/// # Safety
///
/// `plugin_id` must be a valid null-terminated UTF-8 string, or `NULL`.
#[no_mangle]
pub unsafe extern "C" fn webui_handler_create_with_plugin(plugin_id: *const c_char) -> *mut c_void {
    clear_last_error();

    let handler = if plugin_id.is_null() {
        WebUIHandler::new()
    } else {
        match CStr::from_ptr(plugin_id).to_str() {
            Ok("fast" | "fast-v2") => {
                WebUIHandler::with_plugin(|| Box::new(FastV2HydrationPlugin::new()))
            }
            Ok("fast-v3") => WebUIHandler::with_plugin(|| Box::new(FastV3HydrationPlugin::new())),
            Ok("webui") => WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new())),
            Ok(unknown) => {
                set_last_error(format!(
                    "unknown plugin: {unknown}. Use \"webui\", \"fast-v3\", \"fast-v2\", or \"fast\"."
                ));
                return std::ptr::null_mut();
            }
            Err(e) => {
                set_last_error(format!("invalid UTF-8 in plugin_id: {e}"));
                return std::ptr::null_mut();
            }
        }
    };

    let context = Box::new(HandlerContext {
        handler,
        nonce: None,
    });
    Box::into_raw(context) as *mut c_void
}

/// Destroy a WebUI handler instance.
///
/// # Safety
///
/// `handler_ptr` must be a valid pointer returned by [`webui_handler_create`],
/// or `NULL` (in which case this function is a no-op).
#[no_mangle]
pub unsafe extern "C" fn webui_handler_destroy(handler_ptr: *mut c_void) {
    if !handler_ptr.is_null() {
        let _ = Box::from_raw(handler_ptr as *mut HandlerContext);
    }
}

/// Set the CSP nonce for inline `<script>` tags on a handler instance.
///
/// When set, all subsequent renders via [`webui_handler_render`] will include
/// `nonce="VALUE"` on inline script tags and emit a
/// `<meta name="webui-nonce" content="VALUE">` tag in the `<head>`.
///
/// Pass `NULL` to clear a previously set nonce.
///
/// # Thread Safety
///
/// Handler instances are **not** thread-safe. Callers must serialize access
/// to a single `handler_ptr` — do not call `set_nonce` concurrently with
/// `render` or other operations on the same handler.
///
/// # Safety
///
/// * `handler_ptr` must be a valid pointer returned by [`webui_handler_create`].
/// * `nonce` must be a valid null-terminated UTF-8 string, or `NULL`.
/// * Caller must ensure exclusive access to `handler_ptr` (no concurrent calls).
#[no_mangle]
pub unsafe extern "C" fn webui_handler_set_nonce(handler_ptr: *mut c_void, nonce: *const c_char) {
    clear_last_error();

    if handler_ptr.is_null() {
        set_last_error("handler_ptr is null");
        return;
    }

    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: caller guarantees handler_ptr is valid and exclusively owned.
        let context = &mut *(handler_ptr as *mut HandlerContext);

        if nonce.is_null() {
            context.nonce = None;
        } else {
            // SAFETY: caller guarantees nonce is a valid null-terminated string.
            match CStr::from_ptr(nonce).to_str() {
                Ok(s) => context.nonce = Some(s.to_string()),
                Err(e) => {
                    set_last_error(format!("invalid UTF-8 in nonce: {e}"));
                }
            }
        }
    })) {
        Ok(()) => {}
        Err(_) => {
            set_last_error("panic in webui_handler_set_nonce");
        }
    }
}

// ---------------------------------------------------------------------------
// FFI: protobuf-based render (existing API, now with error reporting)
// ---------------------------------------------------------------------------

/// Render a WebUI protocol (protobuf binary) with JSON data.
///
/// # Arguments
///
/// * `handler_ptr`   - Pointer returned by [`webui_handler_create`].
/// * `protocol_data` - Pointer to protobuf binary data.
/// * `protocol_len`  - Length of the protobuf data in bytes.
/// * `data_json`     - Null-terminated JSON string with the render state.
/// * `entry_id`      - Null-terminated UTF-8 string for the entry fragment.
/// * `request_path`  - Null-terminated UTF-8 string for the URL path to match
///   routes against (e.g., `"/contacts/42"`).
///
/// # Returns
///
/// A pointer to a null-terminated UTF-8 string with the rendered HTML, or
/// `NULL` on error.  The caller **must** free the returned string with
/// [`webui_free`].  On error, call [`webui_last_error`] for details.
///
/// # Safety
///
/// * `handler_ptr` must be a valid pointer returned by [`webui_handler_create`].
/// * `protocol_data` must point to `protocol_len` bytes of valid memory.
/// * `data_json`, `entry_id`, and `request_path` must be valid null-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn webui_handler_render(
    handler_ptr: *mut c_void,
    protocol_data: *const u8,
    protocol_len: usize,
    data_json: *const c_char,
    entry_id: *const c_char,
    request_path: *const c_char,
) -> *mut c_char {
    clear_last_error();

    if handler_ptr.is_null()
        || protocol_data.is_null()
        || data_json.is_null()
        || entry_id.is_null()
        || request_path.is_null()
    {
        set_last_error("one or more required arguments are null");
        return std::ptr::null_mut();
    }

    // SAFETY: caller guarantees handler_ptr is valid and exclusively owned.
    let context = &*(handler_ptr as *const HandlerContext);

    // SAFETY: caller guarantees protocol_data points to protocol_len valid bytes.
    let protocol_bytes = std::slice::from_raw_parts(protocol_data, protocol_len);

    // SAFETY: caller guarantees data_json is a valid null-terminated string.
    let data_str = match CStr::from_ptr(data_json).to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("invalid UTF-8 in data_json: {e}"));
            return std::ptr::null_mut();
        }
    };

    // SAFETY: caller guarantees entry_id is a valid null-terminated string.
    let entry_str = match CStr::from_ptr(entry_id).to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("invalid UTF-8 in entry_id: {e}"));
            return std::ptr::null_mut();
        }
    };

    // SAFETY: caller guarantees request_path is a valid null-terminated string.
    let path_str = match CStr::from_ptr(request_path).to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("invalid UTF-8 in request_path: {e}"));
            return std::ptr::null_mut();
        }
    };

    // Parse protocol from protobuf binary data
    let protocol = match WebUIProtocol::from_protobuf(protocol_bytes) {
        Ok(p) => p,
        Err(e) => {
            set_last_error(format!("failed to parse protobuf protocol: {e}"));
            return std::ptr::null_mut();
        }
    };

    // Parse data JSON
    let data: Value = match serde_json::from_str(data_str) {
        Ok(d) => d,
        Err(e) => {
            set_last_error(format!("failed to parse data JSON: {e}"));
            return std::ptr::null_mut();
        }
    };

    // Render
    let mut options = RenderOptions::new(entry_str, path_str);
    if let Some(ref nonce) = context.nonce {
        options = options.with_nonce(nonce);
    }

    let mut writer = StringResponseWriter::new();
    match context
        .handler
        .render(&protocol, &data, &options, &mut writer)
    {
        Ok(_) => match CString::new(writer.content) {
            Ok(s) => s.into_raw(),
            Err(e) => {
                set_last_error(format!("rendered output contains interior NUL byte: {e}"));
                std::ptr::null_mut()
            }
        },
        Err(e) => {
            set_last_error(format!("render failed: {e}"));
            std::ptr::null_mut()
        }
    }
}

// ---------------------------------------------------------------------------
// FFI: HTML-based render (new high-level API)
// ---------------------------------------------------------------------------

/// Parse an HTML template and render it with JSON data in a single call.
///
/// This is the **recommended entry point** for Go, C#, and Python consumers.
/// It eliminates the need for callers to deal with protobuf serialisation.
///
/// Requires the `parser` feature (enabled by default). When built without
/// the `parser` feature, this function always returns `NULL` and sets an
/// error via [`webui_last_error`].
///
/// # Arguments
///
/// * `html`      - Null-terminated UTF-8 string containing the HTML template.
/// * `data_json` - Null-terminated UTF-8 JSON string with the render state.
///
/// # Returns
///
/// A pointer to a null-terminated UTF-8 string with the rendered HTML, or
/// `NULL` on error.  The caller **must** free the returned string with
/// [`webui_free`].  On error, call [`webui_last_error`] for details.
///
/// # Safety
///
/// Both `html` and `data_json` must be valid null-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn webui_render(
    html: *const c_char,
    data_json: *const c_char,
) -> *mut c_char {
    clear_last_error();

    if html.is_null() || data_json.is_null() {
        set_last_error("html and data_json must not be null");
        return std::ptr::null_mut();
    }

    #[cfg(not(feature = "parser"))]
    {
        let _ = (html, data_json);
        set_last_error(
            "webui_render requires the \"parser\" feature, which was not enabled at build time",
        );
        return std::ptr::null_mut();
    }

    #[cfg(feature = "parser")]
    match std::panic::catch_unwind(|| webui_render_impl(html, data_json)) {
        Ok(ptr) => ptr,
        Err(_) => {
            set_last_error("panic in webui_render");
            std::ptr::null_mut()
        }
    }
}

/// Inner implementation of [`webui_render`], compiled only with the `parser` feature.
#[cfg(feature = "parser")]
unsafe fn webui_render_impl(html: *const c_char, data_json: *const c_char) -> *mut c_char {
    // --- Extract C strings ---------------------------------------------------
    // SAFETY: caller (webui_render) already verified non-null.
    let html_str = match CStr::from_ptr(html).to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("invalid UTF-8 in html: {e}"));
            return std::ptr::null_mut();
        }
    };

    // SAFETY: caller (webui_render) already verified non-null.
    let data_str = match CStr::from_ptr(data_json).to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("invalid UTF-8 in data_json: {e}"));
            return std::ptr::null_mut();
        }
    };

    // --- Parse HTML template into a WebUI protocol ---------------------------
    let entry_key = "template";
    let mut parser = HtmlParser::new();
    if let Err(e) = parser.parse(entry_key, html_str) {
        set_last_error(format!("HTML parse error: {e}"));
        return std::ptr::null_mut();
    }

    let protocol = WebUIProtocol::new(parser.into_fragment_records());

    // --- Parse JSON state ----------------------------------------------------
    let data: Value = match serde_json::from_str(data_str) {
        Ok(d) => d,
        Err(e) => {
            set_last_error(format!("failed to parse data JSON: {e}"));
            return std::ptr::null_mut();
        }
    };

    // --- Render --------------------------------------------------------------
    let handler = WebUIHandler::new();
    let mut writer = StringResponseWriter::new();

    match handler.render(
        &protocol,
        &data,
        &RenderOptions::new(entry_key, "/"),
        &mut writer,
    ) {
        Ok(_) => match CString::new(writer.content) {
            Ok(s) => s.into_raw(),
            Err(e) => {
                set_last_error(format!("rendered output contains interior NUL byte: {e}"));
                std::ptr::null_mut()
            }
        },
        Err(e) => {
            set_last_error(format!("render failed: {e}"));
            std::ptr::null_mut()
        }
    }
}

// ---------------------------------------------------------------------------
// FFI: unified partial response
// ---------------------------------------------------------------------------

/// Produce a complete JSON partial response for client-side navigation.
///
/// Combines route templates, inventory, and matched route chain into a single
/// JSON string: `{"templates":[...],"inventory":"...","chain":[...]}`.
///
/// # Arguments
///
/// * `protocol_data` - Pointer to protobuf binary data.
/// * `protocol_len`  - Length of the protobuf data in bytes.
/// * `entry_id`      - Null-terminated UTF-8 string for the persistent entry fragment.
/// * `request_path`  - Null-terminated UTF-8 route path used to select the active route chain.
/// * `inventory_hex` - Null-terminated hex string of the client's inventory bitmask
///   (pass empty string `""` if no inventory).
///
/// # Returns
///
/// A heap-allocated JSON string, or `NULL` on error. Caller frees with [`webui_free`].
///
/// # Safety
///
/// * `protocol_data` must point to `protocol_len` bytes of valid memory.
/// * `state_json`, `entry_id`, `request_path`, and `inventory_hex` must be valid
///   null-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn webui_render_partial(
    protocol_data: *const u8,
    protocol_len: usize,
    state_json: *const c_char,
    entry_id: *const c_char,
    request_path: *const c_char,
    inventory_hex: *const c_char,
) -> *mut c_char {
    clear_last_error();

    match std::panic::catch_unwind(|| {
        if protocol_data.is_null()
            || state_json.is_null()
            || entry_id.is_null()
            || request_path.is_null()
            || inventory_hex.is_null()
        {
            set_last_error("one or more required arguments are null");
            return std::ptr::null_mut();
        }

        // SAFETY: The caller guarantees `protocol_data` points to `protocol_len` readable bytes.
        let protocol_bytes = unsafe { std::slice::from_raw_parts(protocol_data, protocol_len) };

        // SAFETY: The caller guarantees `state_json` is a valid null-terminated string.
        let state_str = match unsafe { CStr::from_ptr(state_json) }.to_str() {
            Ok(s) => s,
            Err(e) => {
                set_last_error(format!("invalid UTF-8 in state_json: {e}"));
                return std::ptr::null_mut();
            }
        };

        let state: Value = match serde_json::from_str(state_str) {
            Ok(v) => v,
            Err(e) => {
                set_last_error(format!("invalid state JSON: {e}"));
                return std::ptr::null_mut();
            }
        };

        // SAFETY: The caller guarantees `entry_id` is a valid null-terminated string.
        let entry_str = match unsafe { CStr::from_ptr(entry_id) }.to_str() {
            Ok(s) => s,
            Err(e) => {
                set_last_error(format!("invalid UTF-8 in entry_id: {e}"));
                return std::ptr::null_mut();
            }
        };

        // SAFETY: The caller guarantees `request_path` is a valid null-terminated string.
        let request_path_str = match unsafe { CStr::from_ptr(request_path) }.to_str() {
            Ok(s) => s,
            Err(e) => {
                set_last_error(format!("invalid UTF-8 in request_path: {e}"));
                return std::ptr::null_mut();
            }
        };

        // SAFETY: The caller guarantees `inventory_hex` is a valid null-terminated string.
        let inv_str = match unsafe { CStr::from_ptr(inventory_hex) }.to_str() {
            Ok(s) => s,
            Err(e) => {
                set_last_error(format!("invalid UTF-8 in inventory_hex: {e}"));
                return std::ptr::null_mut();
            }
        };

        let protocol = match WebUIProtocol::from_protobuf(protocol_bytes) {
            Ok(p) => p,
            Err(e) => {
                set_last_error(format!("failed to parse protobuf protocol: {e}"));
                return std::ptr::null_mut();
            }
        };

        // Per-request index — see ProtocolIndex doc for caching guidance.
        let mut index = webui_handler::route_handler::ProtocolIndex::new(&protocol);

        let mut result = match webui_handler::route_handler::render_partial(
            &protocol,
            entry_str,
            request_path_str,
            inv_str,
            &mut index,
        ) {
            Ok(v) => v,
            Err(e) => {
                set_last_error(format!("render_partial failed: {e}"));
                return std::ptr::null_mut();
            }
        };
        if let Some(obj) = result.as_object_mut() {
            // Broadcast the same shared state object to every chain entry —
            // each component picks only its own @observable keys. JSON
            // serializes the duplicates once and the runtime carries N
            // references to the same object, so memory cost is one pointer
            // slot per entry.
            let chain_len = obj
                .get("chain")
                .and_then(|c| c.as_array())
                .map_or(1, std::vec::Vec::len);
            let mut states = Vec::with_capacity(chain_len);
            for _ in 0..chain_len.saturating_sub(1) {
                states.push(state.clone());
            }
            states.push(state);
            obj.insert("states".into(), serde_json::Value::Array(states));
        }

        match CString::new(result.to_string()) {
            Ok(s) => s.into_raw(),
            Err(e) => {
                set_last_error(format!("JSON output contains interior NUL byte: {e}"));
                std::ptr::null_mut()
            }
        }
    }) {
        Ok(ptr) => ptr,
        Err(_) => {
            set_last_error("panic in webui_render_partial");
            std::ptr::null_mut()
        }
    }
}

/// Render templates and CSS for specific components by tag name.
///
/// Returns a JSON string with `{ templates, templateStyles, cssHrefs, inventory }`.
/// The caller must free the returned string with [`webui_free`].
///
/// # Safety
///
/// All pointer arguments must be valid, non-null, null-terminated UTF-8 strings.
/// `protocol_data` must point to `protocol_len` readable bytes.
/// `component_tags_json` must be a JSON array of strings, e.g. `["settings-dialog"]`.
#[no_mangle]
pub unsafe extern "C" fn webui_render_component_templates(
    protocol_data: *const u8,
    protocol_len: usize,
    component_tags_json: *const c_char,
    inventory_hex: *const c_char,
) -> *mut c_char {
    clear_last_error();

    match std::panic::catch_unwind(|| {
        if protocol_data.is_null() || component_tags_json.is_null() || inventory_hex.is_null() {
            set_last_error("one or more required arguments are null");
            return std::ptr::null_mut();
        }

        let protocol_bytes = unsafe { std::slice::from_raw_parts(protocol_data, protocol_len) };

        let tags_str = match unsafe { CStr::from_ptr(component_tags_json) }.to_str() {
            Ok(s) => s,
            Err(e) => {
                set_last_error(format!("invalid UTF-8 in component_tags_json: {e}"));
                return std::ptr::null_mut();
            }
        };

        let tags: Vec<String> = match serde_json::from_str(tags_str) {
            Ok(v) => v,
            Err(e) => {
                set_last_error(format!("invalid tags JSON: {e}"));
                return std::ptr::null_mut();
            }
        };
        let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();

        let inv_str = match unsafe { CStr::from_ptr(inventory_hex) }.to_str() {
            Ok(s) => s,
            Err(e) => {
                set_last_error(format!("invalid UTF-8 in inventory_hex: {e}"));
                return std::ptr::null_mut();
            }
        };

        let protocol = match WebUIProtocol::from_protobuf(protocol_bytes) {
            Ok(p) => p,
            Err(e) => {
                set_last_error(format!("failed to parse protobuf protocol: {e}"));
                return std::ptr::null_mut();
            }
        };

        // Per-request index — see ProtocolIndex doc for caching guidance.
        let index = webui_handler::route_handler::ProtocolIndex::new(&protocol);

        let result = match webui_handler::route_handler::render_component_templates(
            &protocol, &tag_refs, inv_str, &index,
        ) {
            Ok(v) => v,
            Err(e) => {
                set_last_error(format!("render_component_templates failed: {e}"));
                return std::ptr::null_mut();
            }
        };

        match CString::new(result.to_string()) {
            Ok(s) => s.into_raw(),
            Err(e) => {
                set_last_error(format!("JSON output contains interior NUL byte: {e}"));
                std::ptr::null_mut()
            }
        }
    }) {
        Ok(ptr) => ptr,
        Err(_) => {
            set_last_error("panic in webui_render_component_templates");
            std::ptr::null_mut()
        }
    }
}

/// Free a string returned by a WebUI FFI function.
///
/// # Safety
///
/// `string_ptr` must be a pointer returned by a WebUI FFI function (e.g.
/// [`webui_handler_render`] or [`webui_render`]), or `NULL`
/// (in which case this function is a no-op).
#[no_mangle]
pub unsafe extern "C" fn webui_free(string_ptr: *mut c_char) {
    if !string_ptr.is_null() {
        let _ = CString::from_raw(string_ptr);
    }
}

/// Extract the CSS token name list from a serialized WebUI protocol.
///
/// Returns a heap-allocated newline-delimited string of token names,
/// e.g. `"colorBrandBackground\nfontSizeBase300"`.
///
/// Returns an empty string `""` when the protocol has no tokens.
/// Returns `NULL` only on error (call [`webui_last_error`] for details).
///
/// The caller must free the returned string with [`webui_free`].
///
/// # Safety
///
/// * `protocol_data` must point to `protocol_len` valid bytes.
/// * The returned pointer must be freed with [`webui_free`].
#[no_mangle]
pub unsafe extern "C" fn webui_protocol_tokens(
    protocol_data: *const u8,
    protocol_len: usize,
) -> *mut c_char {
    clear_last_error();

    match std::panic::catch_unwind(|| {
        if protocol_data.is_null() {
            set_last_error("protocol_data is null");
            return std::ptr::null_mut();
        }

        // SAFETY: caller guarantees protocol_data points to protocol_len readable bytes.
        let proto_bytes = unsafe { std::slice::from_raw_parts(protocol_data, protocol_len) };

        let protocol = match WebUIProtocol::from_protobuf(proto_bytes) {
            Ok(p) => p,
            Err(e) => {
                set_last_error(format!("failed to parse protobuf: {e}"));
                return std::ptr::null_mut();
            }
        };

        // join() on an empty vec produces "", which is a valid success result.
        let joined = protocol.tokens.join("\n");

        match CString::new(joined) {
            Ok(cs) => cs.into_raw(),
            Err(e) => {
                set_last_error(format!("token string contains null byte: {e}"));
                std::ptr::null_mut()
            }
        }
    }) {
        Ok(ptr) => ptr,
        Err(_) => {
            set_last_error("panic in webui_protocol_tokens");
            std::ptr::null_mut()
        }
    }
}
