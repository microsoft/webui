// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// FFI crate requires unsafe for C-compatible ABI boundary.
#![allow(unsafe_code)]

//! WebUI FFI (Foreign Function Interface) for interoperability with other languages.
//!
//! This crate provides C-compatible APIs for the WebUI handler to be used from languages
//! like Go, C#, Python, etc.
//!
//! Load a compiled protocol once with [`webui_protocol_create`], then reuse the
//! handle with [`webui_handler_render`] and the other protocol operations.
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
use webui_handler::{Protocol, RenderOptions, ResponseWriter, WebUIHandler};

/// Opaque C handle for a loaded WebUI protocol.
#[allow(non_camel_case_types)]
pub type webui_protocol_t = c_void;

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

/// Opaque decoded protocol context shared across repeated host calls.
struct ProtocolContext {
    protocol: Protocol,
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

/// Decode and index a WebUI protocol for repeated rendering.
///
/// The returned handle is thread-safe and must be released with
/// [`webui_protocol_destroy`].
///
/// # Safety
///
/// `protocol_data` must point to `protocol_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn webui_protocol_create(
    protocol_data: *const u8,
    protocol_len: usize,
) -> *mut webui_protocol_t {
    clear_last_error();
    if protocol_data.is_null() {
        set_last_error("protocol_data is null");
        return std::ptr::null_mut();
    }

    match std::panic::catch_unwind(|| {
        // SAFETY: The caller guarantees that the input range is readable.
        let bytes = unsafe { std::slice::from_raw_parts(protocol_data, protocol_len) };
        match Protocol::from_protobuf(bytes) {
            Ok(protocol) => {
                Box::into_raw(Box::new(ProtocolContext { protocol })) as *mut webui_protocol_t
            }
            Err(error) => {
                set_last_error(format!("failed to parse protobuf protocol: {error}"));
                std::ptr::null_mut()
            }
        }
    }) {
        Ok(ptr) => ptr,
        Err(_) => {
            set_last_error("panic in webui_protocol_create");
            std::ptr::null_mut()
        }
    }
}

/// Destroy a loaded WebUI protocol handle.
///
/// # Safety
///
/// `protocol_ptr` must be a pointer returned by [`webui_protocol_create`], or
/// `NULL` for a no-op.
#[no_mangle]
pub unsafe extern "C" fn webui_protocol_destroy(protocol_ptr: *mut webui_protocol_t) {
    if !protocol_ptr.is_null() {
        // SAFETY: The caller guarantees this pointer came from
        // `webui_protocol_create` and has not already been destroyed.
        let _ = unsafe { Box::from_raw(protocol_ptr as *mut ProtocolContext) };
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
/// Concurrent render calls are supported after configuration. Callers must not
/// call `set_nonce` or destroy the handler concurrently with any operation on
/// the same `handler_ptr`.
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
// FFI: protocol rendering
// ---------------------------------------------------------------------------

/// Render using a protocol previously returned by [`webui_protocol_create`].
///
/// # Safety
///
/// * `handler_ptr` must be a valid handler pointer.
/// * `protocol_ptr` must be a valid loaded protocol pointer.
/// * String arguments must be valid null-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn webui_handler_render(
    handler_ptr: *mut c_void,
    protocol_ptr: *const webui_protocol_t,
    data_json: *const c_char,
    entry_id: *const c_char,
    request_path: *const c_char,
) -> *mut c_char {
    clear_last_error();

    match std::panic::catch_unwind(|| {
        if handler_ptr.is_null()
            || protocol_ptr.is_null()
            || data_json.is_null()
            || entry_id.is_null()
            || request_path.is_null()
        {
            set_last_error("one or more required arguments are null");
            return std::ptr::null_mut();
        }

        // SAFETY: The caller guarantees both opaque pointers are valid.
        let context = unsafe { &*(handler_ptr as *const HandlerContext) };
        let protocol_context = unsafe { &*(protocol_ptr as *const ProtocolContext) };
        // SAFETY: The caller guarantees all string pointers are valid.
        unsafe {
            render_decoded_protocol(
                context,
                &protocol_context.protocol,
                data_json,
                entry_id,
                request_path,
            )
        }
    }) {
        Ok(ptr) => ptr,
        Err(_) => {
            set_last_error("panic in webui_handler_render");
            std::ptr::null_mut()
        }
    }
}

unsafe fn render_decoded_protocol(
    context: &HandlerContext,
    protocol: &Protocol,
    data_json: *const c_char,
    entry_id: *const c_char,
    request_path: *const c_char,
) -> *mut c_char {
    // SAFETY: The caller validates all pointers before invoking this helper.
    let data_str = match unsafe { CStr::from_ptr(data_json) }.to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("invalid UTF-8 in data_json: {e}"));
            return std::ptr::null_mut();
        }
    };
    // SAFETY: The caller validates all pointers before invoking this helper.
    let entry_str = match unsafe { CStr::from_ptr(entry_id) }.to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("invalid UTF-8 in entry_id: {e}"));
            return std::ptr::null_mut();
        }
    };
    // SAFETY: The caller validates all pointers before invoking this helper.
    let path_str = match unsafe { CStr::from_ptr(request_path) }.to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("invalid UTF-8 in request_path: {e}"));
            return std::ptr::null_mut();
        }
    };

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
        .render(protocol, &data, &options, &mut writer)
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
// FFI: unified partial response
// ---------------------------------------------------------------------------

/// Produce a complete partial response using a loaded protocol handle.
///
/// # Safety
///
/// * `protocol_ptr` must be a valid pointer returned by [`webui_protocol_create`].
/// * All string pointers must be valid, non-null, null-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn webui_protocol_render_partial(
    protocol_ptr: *const webui_protocol_t,
    state_json: *const c_char,
    entry_id: *const c_char,
    request_path: *const c_char,
    inventory_hex: *const c_char,
) -> *mut c_char {
    clear_last_error();

    match std::panic::catch_unwind(|| {
        if protocol_ptr.is_null()
            || state_json.is_null()
            || entry_id.is_null()
            || request_path.is_null()
            || inventory_hex.is_null()
        {
            set_last_error("one or more required arguments are null");
            return std::ptr::null_mut();
        }

        // SAFETY: The caller guarantees that all opaque/string pointers are valid.
        let protocol_context = unsafe { &*(protocol_ptr as *const ProtocolContext) };
        let state_str = match unsafe { CStr::from_ptr(state_json) }.to_str() {
            Ok(value) => value,
            Err(error) => {
                set_last_error(format!("invalid UTF-8 in state_json: {error}"));
                return std::ptr::null_mut();
            }
        };
        let entry_str = match unsafe { CStr::from_ptr(entry_id) }.to_str() {
            Ok(value) => value,
            Err(error) => {
                set_last_error(format!("invalid UTF-8 in entry_id: {error}"));
                return std::ptr::null_mut();
            }
        };
        let request_path_str = match unsafe { CStr::from_ptr(request_path) }.to_str() {
            Ok(value) => value,
            Err(error) => {
                set_last_error(format!("invalid UTF-8 in request_path: {error}"));
                return std::ptr::null_mut();
            }
        };
        let inventory_str = match unsafe { CStr::from_ptr(inventory_hex) }.to_str() {
            Ok(value) => value,
            Err(error) => {
                set_last_error(format!("invalid UTF-8 in inventory_hex: {error}"));
                return std::ptr::null_mut();
            }
        };

        let output = match protocol_context.protocol.render_partial(
            state_str,
            entry_str,
            request_path_str,
            inventory_str,
        ) {
            Ok(value) => value,
            Err(error) => {
                set_last_error(format!("render_partial failed: {error}"));
                return std::ptr::null_mut();
            }
        };

        match CString::new(output) {
            Ok(value) => value.into_raw(),
            Err(error) => {
                set_last_error(format!("JSON output contains interior NUL byte: {error}"));
                std::ptr::null_mut()
            }
        }
    }) {
        Ok(ptr) => ptr,
        Err(_) => {
            set_last_error("panic in webui_protocol_render_partial");
            std::ptr::null_mut()
        }
    }
}

/// Render component templates using a loaded protocol handle.
///
/// # Safety
///
/// * `protocol_ptr` must be a valid pointer returned by [`webui_protocol_create`].
/// * String arguments must be valid, non-null, null-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn webui_protocol_render_component_templates(
    protocol_ptr: *const webui_protocol_t,
    component_tags_json: *const c_char,
    inventory_hex: *const c_char,
) -> *mut c_char {
    clear_last_error();

    match std::panic::catch_unwind(|| {
        if protocol_ptr.is_null() || component_tags_json.is_null() || inventory_hex.is_null() {
            set_last_error("one or more required arguments are null");
            return std::ptr::null_mut();
        }

        // SAFETY: The caller guarantees that all opaque/string pointers are valid.
        let protocol_context = unsafe { &*(protocol_ptr as *const ProtocolContext) };
        let tags_str = match unsafe { CStr::from_ptr(component_tags_json) }.to_str() {
            Ok(value) => value,
            Err(error) => {
                set_last_error(format!("invalid UTF-8 in component_tags_json: {error}"));
                return std::ptr::null_mut();
            }
        };
        let tags: Vec<String> = match serde_json::from_str(tags_str) {
            Ok(value) => value,
            Err(error) => {
                set_last_error(format!("invalid tags JSON: {error}"));
                return std::ptr::null_mut();
            }
        };
        let tag_refs: Vec<&str> = tags.iter().map(String::as_str).collect();
        let inventory_str = match unsafe { CStr::from_ptr(inventory_hex) }.to_str() {
            Ok(value) => value,
            Err(error) => {
                set_last_error(format!("invalid UTF-8 in inventory_hex: {error}"));
                return std::ptr::null_mut();
            }
        };

        let result = match protocol_context
            .protocol
            .render_component_templates(&tag_refs, inventory_str)
        {
            Ok(value) => value,
            Err(error) => {
                set_last_error(format!("render_component_templates failed: {error}"));
                return std::ptr::null_mut();
            }
        };

        match CString::new(result.to_string()) {
            Ok(value) => value.into_raw(),
            Err(error) => {
                set_last_error(format!("JSON output contains interior NUL byte: {error}"));
                std::ptr::null_mut()
            }
        }
    }) {
        Ok(ptr) => ptr,
        Err(_) => {
            set_last_error("panic in webui_protocol_render_component_templates");
            std::ptr::null_mut()
        }
    }
}

/// Free a string returned by a WebUI FFI function.
///
/// # Safety
///
/// `string_ptr` must be a pointer returned by a WebUI FFI function such as
/// [`webui_handler_render`], or `NULL`
/// (in which case this function is a no-op).
#[no_mangle]
pub unsafe extern "C" fn webui_free(string_ptr: *mut c_char) {
    if !string_ptr.is_null() {
        let _ = CString::from_raw(string_ptr);
    }
}

/// Extract CSS token names from a loaded protocol handle.
///
/// Returns a newline-delimited representation.
///
/// # Safety
///
/// * `protocol_ptr` must be a valid pointer returned by [`webui_protocol_create`].
/// * The returned pointer must be freed with [`webui_free`].
#[no_mangle]
pub unsafe extern "C" fn webui_protocol_tokens(
    protocol_ptr: *const webui_protocol_t,
) -> *mut c_char {
    clear_last_error();

    match std::panic::catch_unwind(|| {
        if protocol_ptr.is_null() {
            set_last_error("protocol_ptr is null");
            return std::ptr::null_mut();
        }

        // SAFETY: The caller guarantees protocol_ptr is a live loaded handle.
        let protocol_context = unsafe { &*(protocol_ptr as *const ProtocolContext) };
        let joined = protocol_context.protocol.tokens().join("\n");

        match CString::new(joined) {
            Ok(value) => value.into_raw(),
            Err(error) => {
                set_last_error(format!("token string contains null byte: {error}"));
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
