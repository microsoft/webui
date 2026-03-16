// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

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
use webui_handler::plugin::FastHydrationPlugin;
use webui_handler::{RenderOptions, ResponseWriter, WebUIHandler};
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
    let context = Box::new(HandlerContext { handler });
    Box::into_raw(context) as *mut c_void
}

/// Create a new WebUI handler instance with a named plugin.
///
/// # Arguments
///
/// * `plugin_id` - Null-terminated UTF-8 string identifying the plugin.
///   Currently supported: `"fast"`. Pass `NULL` for no plugin.
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
            Ok("fast") => WebUIHandler::with_plugin(|| Box::new(FastHydrationPlugin::new())),
            Ok(unknown) => {
                set_last_error(format!("unknown plugin: {unknown}"));
                return std::ptr::null_mut();
            }
            Err(e) => {
                set_last_error(format!("invalid UTF-8 in plugin_id: {e}"));
                return std::ptr::null_mut();
            }
        }
    };

    let context = Box::new(HandlerContext { handler });
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
    let mut writer = StringResponseWriter::new();
    match context.handler.render(
        &protocol,
        &data,
        &RenderOptions::new(entry_str, path_str),
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
// FFI: HTML-based render (new high-level API)
// ---------------------------------------------------------------------------

/// Parse an HTML template and render it with JSON data in a single call.
///
/// This is the **recommended entry point** for Go, C#, and Python consumers.
/// It eliminates the need for callers to deal with protobuf serialisation.
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

    // --- Extract C strings ---------------------------------------------------
    let html_str = match CStr::from_ptr(html).to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("invalid UTF-8 in html: {e}"));
            return std::ptr::null_mut();
        }
    };

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
// FFI: route template query
// ---------------------------------------------------------------------------

/// Get the f-template HTML strings needed for the active route chain.
///
/// Walks the protocol's fragment graph from the persistent `entry_id` root,
/// follows only the best-matching nested route chain for `request_path`,
/// identifies components not in the client's `inventory_hex` bitmask, and
/// returns a JSON string:
/// `{"templates":[{"name":"...","html":"..."}...],"inventory":"..."}`.
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
/// * `entry_id`, `request_path`, and `inventory_hex` must be valid null-terminated UTF-8
///   strings.
#[no_mangle]
pub unsafe extern "C" fn webui_get_route_templates(
    protocol_data: *const u8,
    protocol_len: usize,
    entry_id: *const c_char,
    request_path: *const c_char,
    inventory_hex: *const c_char,
) -> *mut c_char {
    clear_last_error();

    match std::panic::catch_unwind(|| {
        if protocol_data.is_null()
            || entry_id.is_null()
            || request_path.is_null()
            || inventory_hex.is_null()
        {
            set_last_error("one or more required arguments are null");
            return std::ptr::null_mut();
        }

        // SAFETY: The caller guarantees `protocol_data` points to `protocol_len` readable bytes.
        let protocol_bytes = unsafe { std::slice::from_raw_parts(protocol_data, protocol_len) };

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

        let (templates, updated_inv) =
            webui_handler::route_handler::get_route_templates_for_request(
                &protocol,
                entry_str,
                request_path_str,
                inv_str,
            );

        // Build JSON response without json! macro (which uses unwrap internally)
        let tmpl_array: Vec<Value> = templates
            .iter()
            .map(|(name, html)| {
                let mut obj = serde_json::Map::with_capacity(2);
                obj.insert("name".into(), Value::String(name.clone()));
                obj.insert("html".into(), Value::String(html.clone()));
                Value::Object(obj)
            })
            .collect();

        let mut result = serde_json::Map::with_capacity(2);
        result.insert("templates".into(), Value::Array(tmpl_array));
        result.insert("inventory".into(), Value::String(updated_inv));

        match CString::new(Value::Object(result).to_string()) {
            Ok(s) => s.into_raw(),
            Err(e) => {
                set_last_error(format!("JSON output contains interior NUL byte: {e}"));
                std::ptr::null_mut()
            }
        }
    }) {
        Ok(ptr) => ptr,
        Err(_) => {
            set_last_error("panic in webui_get_route_templates");
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
