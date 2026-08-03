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
use std::sync::Arc;
use webui_handler::plugin::fast_v2::FastV2HydrationPlugin;
use webui_handler::plugin::fast_v3::FastV3HydrationPlugin;
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::{
    BoundaryId, BoundaryMode, Protocol, RenderOptions, ResponseWriter, SessionOptions,
    StreamingSession, WebUIHandler,
};

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
    handler: Arc<WebUIHandler>,
    /// CSP nonce for inline `<script>` tags (set via `webui_handler_set_nonce`).
    nonce: Option<String>,
}

/// Opaque decoded protocol context shared across repeated host calls.
struct ProtocolContext {
    protocol: Arc<Protocol>,
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
        handler: Arc::new(handler),
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
        handler: Arc::new(handler),
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
            Ok(protocol) => Box::into_raw(Box::new(ProtocolContext {
                protocol: Arc::new(protocol),
            })) as *mut webui_protocol_t,
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

// ---------------------------------------------------------------------------
// FFI: host-driven streaming sessions
// ---------------------------------------------------------------------------

/// Opaque C handle for a host-driven progressive response.
#[allow(non_camel_case_types)]
pub type webui_streaming_session_t = c_void;

/// Owns one progressive response between host calls.
struct StreamingSessionContext {
    session: StreamingSession,
}

/// Open a host-driven progressive response for a streaming entry.
///
/// Unlike [`webui_handler_render`], which produces the whole document in one
/// call, the returned session hands back one chunk per call so the host owns
/// the socket, the write order, and backpressure. Any nonce previously set
/// with [`webui_handler_set_nonce`] is captured for the life of the session.
///
/// Returns `NULL` on error; call [`webui_last_error`] for details. The handle
/// must be released with [`webui_streaming_session_destroy`] even after
/// [`webui_streaming_session_finish`] succeeds.
///
/// # Thread Safety
///
/// A session is **not** thread-safe. Drive one session from one thread at a
/// time. Independent sessions may run concurrently on the same handler and
/// protocol.
///
/// # Safety
///
/// * `handler_ptr` must be a valid pointer from [`webui_handler_create`].
/// * `protocol_ptr` must be a valid pointer from [`webui_protocol_create`].
/// * `entry_id` and `request_path` must be non-null, null-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn webui_streaming_session_create(
    handler_ptr: *mut c_void,
    protocol_ptr: *const webui_protocol_t,
    entry_id: *const c_char,
    request_path: *const c_char,
) -> *mut webui_streaming_session_t {
    clear_last_error();

    match std::panic::catch_unwind(|| {
        if handler_ptr.is_null()
            || protocol_ptr.is_null()
            || entry_id.is_null()
            || request_path.is_null()
        {
            set_last_error("one or more required arguments are null");
            return std::ptr::null_mut();
        }

        // SAFETY: The caller guarantees both opaque pointers are live.
        let handler_context = unsafe { &*(handler_ptr as *const HandlerContext) };
        let protocol_context = unsafe { &*(protocol_ptr as *const ProtocolContext) };

        // SAFETY: The caller guarantees both strings are valid and terminated.
        let Some(entry) = (unsafe { utf8_arg(entry_id, "entry_id") }) else {
            return std::ptr::null_mut();
        };
        // SAFETY: The caller guarantees both strings are valid and terminated.
        let Some(path) = (unsafe { utf8_arg(request_path, "request_path") }) else {
            return std::ptr::null_mut();
        };

        let mut options = SessionOptions::new(entry, path);
        options.nonce = handler_context.nonce.clone();

        match StreamingSession::new(
            Arc::clone(&handler_context.handler),
            Arc::clone(&protocol_context.protocol),
            options,
        ) {
            Ok(session) => Box::into_raw(Box::new(StreamingSessionContext { session }))
                as *mut webui_streaming_session_t,
            Err(error) => {
                set_last_error(error.to_string());
                std::ptr::null_mut()
            }
        }
    }) {
        Ok(ptr) => ptr,
        Err(_) => {
            set_last_error("panic in webui_streaming_session_create");
            std::ptr::null_mut()
        }
    }
}

/// Release a streaming session handle.
///
/// Safe to call on an unfinished session; any buffered bytes are dropped.
///
/// # Safety
///
/// `session_ptr` must be a pointer returned by
/// [`webui_streaming_session_create`], or `NULL` for a no-op. It must not be
/// used again afterwards.
#[no_mangle]
pub unsafe extern "C" fn webui_streaming_session_destroy(
    session_ptr: *mut webui_streaming_session_t,
) {
    if !session_ptr.is_null() {
        // SAFETY: The caller guarantees this pointer came from
        // `webui_streaming_session_create` and has not already been destroyed.
        let _ = unsafe { Box::from_raw(session_ptr as *mut StreamingSessionContext) };
    }
}

/// Resolve an authored boundary name to a stable integer handle.
///
/// Resolve once outside the write loop and reuse the handle; the write calls
/// never hash a name.
///
/// Returns `true` on success and writes the handle to `out_boundary`. On
/// failure returns `false` and leaves `out_boundary` untouched; call
/// [`webui_last_error`] for the valid names and a suggestion.
///
/// # Safety
///
/// * `session_ptr` must be a live session handle.
/// * `name` must be non-null, null-terminated UTF-8.
/// * `out_boundary` must be non-null and writable.
#[no_mangle]
pub unsafe extern "C" fn webui_streaming_session_boundary(
    session_ptr: *const webui_streaming_session_t,
    name: *const c_char,
    out_boundary: *mut u32,
) -> bool {
    clear_last_error();

    match std::panic::catch_unwind(|| {
        if session_ptr.is_null() || name.is_null() || out_boundary.is_null() {
            set_last_error("one or more required arguments are null");
            return false;
        }

        // SAFETY: The caller guarantees the session handle is live.
        let context = unsafe { &*(session_ptr as *const StreamingSessionContext) };
        // SAFETY: The caller guarantees `name` is valid and terminated.
        let Some(name) = (unsafe { utf8_arg(name, "name") }) else {
            return false;
        };

        match context.session.boundary(name) {
            Ok(boundary) => {
                // SAFETY: The caller guarantees `out_boundary` is writable.
                unsafe { *out_boundary = boundary.raw() };
                true
            }
            Err(error) => {
                set_last_error(error.to_string());
                false
            }
        }
    }) {
        Ok(ok) => ok,
        Err(_) => {
            set_last_error("panic in webui_streaming_session_boundary");
            false
        }
    }
}

/// Return the number of compile-time boundaries declared by this entry.
///
/// Returns `0` for a `NULL` handle.
///
/// # Safety
///
/// `session_ptr` must be a live session handle, or `NULL`.
#[no_mangle]
pub unsafe extern "C" fn webui_streaming_session_boundary_count(
    session_ptr: *const webui_streaming_session_t,
) -> u32 {
    if session_ptr.is_null() {
        return 0;
    }
    // SAFETY: The caller guarantees the session handle is live.
    let context = unsafe { &*(session_ptr as *const StreamingSessionContext) };
    u32::try_from(context.session.boundary_count()).unwrap_or(u32::MAX)
}

/// Report whether the terminal record has been written.
///
/// Returns `true` for a `NULL` handle, because a session that does not exist
/// can never accept another call.
///
/// # Safety
///
/// `session_ptr` must be a live session handle, or `NULL`.
#[no_mangle]
pub unsafe extern "C" fn webui_streaming_session_is_finished(
    session_ptr: *const webui_streaming_session_t,
) -> bool {
    if session_ptr.is_null() {
        return true;
    }
    // SAFETY: The caller guarantees the session handle is live.
    let context = unsafe { &*(session_ptr as *const StreamingSessionContext) };
    context.session.is_finished()
}

/// Render everything before the first boundary.
///
/// Returns a NUL-terminated UTF-8 chunk that must be freed with
/// [`webui_free`], or `NULL` on error. When `out_len` is non-null it receives
/// the byte length excluding the terminator, so hosts writing to a socket do
/// not need `strlen`.
///
/// # Safety
///
/// * `session_ptr` must be a live session handle.
/// * `state_json` must be non-null, null-terminated UTF-8.
/// * `out_len` must be writable, or `NULL`.
#[no_mangle]
pub unsafe extern "C" fn webui_streaming_session_write_shell(
    session_ptr: *mut webui_streaming_session_t,
    state_json: *const c_char,
    out_len: *mut usize,
) -> *mut c_char {
    // SAFETY: Forwarded verbatim; this helper repeats every check.
    unsafe {
        streaming_chunk_call(
            session_ptr,
            state_json,
            out_len,
            "webui_streaming_session_write_shell",
            |session, state| session.write_shell(state),
        )
    }
}

/// Render and commit the next boundary in declaration order.
///
/// Pass `updatable = true` only for boundaries you intend to patch later with
/// [`webui_streaming_session_update`]; an updatable boundary retains its roots
/// and projection until the terminal record.
///
/// Returns a NUL-terminated UTF-8 chunk that must be freed with
/// [`webui_free`], or `NULL` on error. When `out_len` is non-null it receives
/// the byte length excluding the terminator.
///
/// # Safety
///
/// * `session_ptr` must be a live session handle.
/// * `state_json` must be non-null, null-terminated UTF-8.
/// * `out_len` must be writable, or `NULL`.
#[no_mangle]
pub unsafe extern "C" fn webui_streaming_session_write_boundary(
    session_ptr: *mut webui_streaming_session_t,
    boundary: u32,
    state_json: *const c_char,
    updatable: bool,
    out_len: *mut usize,
) -> *mut c_char {
    let mode = if updatable {
        BoundaryMode::Updatable
    } else {
        BoundaryMode::Final
    };
    // SAFETY: Forwarded verbatim; this helper repeats every check.
    unsafe {
        streaming_chunk_call(
            session_ptr,
            state_json,
            out_len,
            "webui_streaming_session_write_boundary",
            |session, state| session.write_boundary(BoundaryId::from_raw(boundary), state, mode),
        )
    }
}

/// Push a projected state patch to a committed updatable boundary.
///
/// Returns a NUL-terminated UTF-8 chunk that must be freed with
/// [`webui_free`], or `NULL` on error. When `out_len` is non-null it receives
/// the byte length excluding the terminator.
///
/// # Safety
///
/// * `session_ptr` must be a live session handle.
/// * `state_json` must be non-null, null-terminated UTF-8.
/// * `out_len` must be writable, or `NULL`.
#[no_mangle]
pub unsafe extern "C" fn webui_streaming_session_update(
    session_ptr: *mut webui_streaming_session_t,
    boundary: u32,
    state_json: *const c_char,
    out_len: *mut usize,
) -> *mut c_char {
    // SAFETY: Forwarded verbatim; this helper repeats every check.
    unsafe {
        streaming_chunk_call(
            session_ptr,
            state_json,
            out_len,
            "webui_streaming_session_update",
            |session, state| session.update(BoundaryId::from_raw(boundary), state),
        )
    }
}

/// Render the document tail and emit the terminal record.
///
/// Every later call fails. The handle must still be released with
/// [`webui_streaming_session_destroy`].
///
/// Returns a NUL-terminated UTF-8 chunk that must be freed with
/// [`webui_free`], or `NULL` on error. When `out_len` is non-null it receives
/// the byte length excluding the terminator.
///
/// # Safety
///
/// * `session_ptr` must be a live session handle.
/// * `state_json` must be non-null, null-terminated UTF-8.
/// * `out_len` must be writable, or `NULL`.
#[no_mangle]
pub unsafe extern "C" fn webui_streaming_session_finish(
    session_ptr: *mut webui_streaming_session_t,
    state_json: *const c_char,
    out_len: *mut usize,
) -> *mut c_char {
    // SAFETY: Forwarded verbatim; this helper repeats every check.
    unsafe {
        streaming_chunk_call(
            session_ptr,
            state_json,
            out_len,
            "webui_streaming_session_finish",
            |session, state| session.finish(state),
        )
    }
}

/// Shared body for every chunk-producing session call.
///
/// # Safety
///
/// Pointers must satisfy the contract documented on the calling function.
unsafe fn streaming_chunk_call(
    session_ptr: *mut webui_streaming_session_t,
    state_json: *const c_char,
    out_len: *mut usize,
    operation: &str,
    render: impl FnOnce(&mut StreamingSession, &Value) -> webui_handler::Result<Vec<u8>>,
) -> *mut c_char {
    clear_last_error();

    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if session_ptr.is_null() || state_json.is_null() {
            set_last_error("one or more required arguments are null");
            return std::ptr::null_mut();
        }

        // SAFETY: The caller guarantees exclusive access to a live session.
        let context = unsafe { &mut *(session_ptr as *mut StreamingSessionContext) };
        // SAFETY: The caller guarantees `state_json` is valid and terminated.
        let Some(state_str) = (unsafe { utf8_arg(state_json, "state_json") }) else {
            return std::ptr::null_mut();
        };

        let state: Value = match serde_json::from_str(state_str) {
            Ok(value) => value,
            Err(error) => {
                set_last_error(format!("failed to parse state JSON: {error}"));
                return std::ptr::null_mut();
            }
        };

        let bytes = match render(&mut context.session, &state) {
            Ok(bytes) => bytes,
            Err(error) => {
                set_last_error(error.to_string());
                return std::ptr::null_mut();
            }
        };

        let length = bytes.len();
        match CString::new(bytes) {
            Ok(chunk) => {
                if !out_len.is_null() {
                    // SAFETY: The caller guarantees `out_len` is writable.
                    unsafe { *out_len = length };
                }
                chunk.into_raw()
            }
            Err(error) => {
                set_last_error(format!("chunk contains interior NUL byte: {error}"));
                std::ptr::null_mut()
            }
        }
    })) {
        Ok(ptr) => ptr,
        Err(_) => {
            set_last_error(format!("panic in {operation}"));
            std::ptr::null_mut()
        }
    }
}

/// Borrow a C string argument as UTF-8, recording an actionable error on failure.
///
/// # Safety
///
/// `value` must be non-null, null-terminated, and live for the returned borrow.
unsafe fn utf8_arg<'a>(value: *const c_char, name: &str) -> Option<&'a str> {
    // SAFETY: The caller guarantees the pointer is valid and terminated.
    match unsafe { CStr::from_ptr(value) }.to_str() {
        Ok(text) => Some(text),
        Err(error) => {
            set_last_error(format!("invalid UTF-8 in {name}: {error}"));
            None
        }
    }
}
