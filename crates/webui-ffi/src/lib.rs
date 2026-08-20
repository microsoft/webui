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
    BoundaryDescriptor, BoundaryInstanceId, BoundaryKey, BoundaryMode, Protocol, RenderOptions,
    ResponseWriter, SessionOptions, StreamStep, StreamingSession, WebUIHandler,
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

/// Opaque owned result from one streaming start or resume call.
#[allow(non_camel_case_types)]
pub type webui_streaming_step_t = c_void;

/// C-safe boundary mode value accepted by [`webui_streaming_session_resume`].
#[allow(non_camel_case_types)]
pub type webui_boundary_mode_t = u32;

/// Commit the boundary once and release its boundary-local roots.
pub const WEBUI_BOUNDARY_MODE_FINAL: webui_boundary_mode_t = 0;

/// Retain live roots until terminal so updates may target the boundary.
pub const WEBUI_BOUNDARY_MODE_UPDATABLE: webui_boundary_mode_t = 1;

/// C-safe boundary key discriminator returned by
/// [`webui_streaming_step_boundary_key_type`].
#[allow(non_camel_case_types)]
pub type webui_boundary_key_type_t = u32;

/// The boundary declaration has no runtime key.
pub const WEBUI_BOUNDARY_KEY_NONE: webui_boundary_key_type_t = 0;

/// The boundary key is a UTF-8 string.
pub const WEBUI_BOUNDARY_KEY_STRING: webui_boundary_key_type_t = 1;

/// The boundary key is a finite JSON number.
pub const WEBUI_BOUNDARY_KEY_NUMBER: webui_boundary_key_type_t = 2;

/// Owns one progressive response between host calls.
struct StreamingSessionContext {
    session: StreamingSession,
}

/// Owns one result and every pointer borrowed from it.
struct StreamingStepContext {
    step: StreamStep,
}

/// Open a host-driven progressive response for a streaming entry.
///
/// Unlike [`webui_handler_render`], which produces the whole document in one
/// call, the returned session advances through [`webui_streaming_session_start`]
/// and [`webui_streaming_session_resume`] so the host owns the socket, write
/// order, and backpressure. Any nonce previously set with
/// [`webui_handler_set_nonce`] is captured for the life of the session.
///
/// Returns `NULL` on error; call [`webui_last_error`] for details. The handle
/// must be released with [`webui_streaming_session_destroy`].
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
    clear_last_error();
    if session_ptr.is_null() {
        return;
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: The caller guarantees this pointer came from
        // `webui_streaming_session_create` and has not already been destroyed.
        drop(unsafe { Box::from_raw(session_ptr as *mut StreamingSessionContext) });
    }));
    if result.is_err() {
        set_last_error("panic in webui_streaming_session_destroy");
    }
}

/// Render until the first runtime boundary occurrence or terminal completion.
///
/// The returned owned step must be released with
/// [`webui_streaming_step_destroy`]. A `NULL` return indicates an error
/// available through [`webui_last_error`].
///
/// # Safety
///
/// * `session_ptr` must be a live session handle with no concurrent operation.
/// * `state_json` must be non-null, null-terminated UTF-8 and remain readable
///   for this call.
#[no_mangle]
pub unsafe extern "C" fn webui_streaming_session_start(
    session_ptr: *mut webui_streaming_session_t,
    state_json: *const c_char,
) -> *mut webui_streaming_step_t {
    clear_last_error();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if session_ptr.is_null() || state_json.is_null() {
            set_last_error("session_ptr and state_json must not be null");
            return std::ptr::null_mut();
        }
        // SAFETY: The caller guarantees exclusive access to a live session.
        let context = unsafe { &mut *(session_ptr as *mut StreamingSessionContext) };
        // SAFETY: The caller guarantees the string is readable and terminated.
        let Some(state) = (unsafe { streaming_json_arg(state_json, "state_json") }) else {
            return std::ptr::null_mut();
        };
        match context.session.start(&state) {
            Ok(step) => owned_streaming_step(step),
            Err(error) => {
                set_last_error(error.to_string());
                std::ptr::null_mut()
            }
        }
    })) {
        Ok(step) => step,
        Err(_) => {
            set_last_error("panic in webui_streaming_session_start");
            std::ptr::null_mut()
        }
    }
}

/// Commit the pending occurrence and advance to the next occurrence or terminal.
///
/// `mode` must be [`WEBUI_BOUNDARY_MODE_FINAL`] or
/// [`WEBUI_BOUNDARY_MODE_UPDATABLE`]. The returned owned step must be released
/// with [`webui_streaming_step_destroy`]. A `NULL` return indicates an error
/// available through [`webui_last_error`].
///
/// # Safety
///
/// * `session_ptr` must be a live session handle with no concurrent operation.
/// * `state_json` must be non-null, null-terminated UTF-8 and remain readable
///   for this call.
#[no_mangle]
pub unsafe extern "C" fn webui_streaming_session_resume(
    session_ptr: *mut webui_streaming_session_t,
    instance_id: u32,
    state_json: *const c_char,
    mode: webui_boundary_mode_t,
) -> *mut webui_streaming_step_t {
    clear_last_error();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if session_ptr.is_null() || state_json.is_null() {
            set_last_error("session_ptr and state_json must not be null");
            return std::ptr::null_mut();
        }
        let Some(mode) = streaming_boundary_mode(mode) else {
            return std::ptr::null_mut();
        };
        // SAFETY: The caller guarantees exclusive access to a live session.
        let context = unsafe { &mut *(session_ptr as *mut StreamingSessionContext) };
        // SAFETY: The caller guarantees the string is readable and terminated.
        let Some(state) = (unsafe { streaming_json_arg(state_json, "state_json") }) else {
            return std::ptr::null_mut();
        };
        match context
            .session
            .resume(BoundaryInstanceId::from_raw(instance_id), &state, mode)
        {
            Ok(step) => owned_streaming_step(step),
            Err(error) => {
                set_last_error(error.to_string());
                std::ptr::null_mut()
            }
        }
    })) {
        Ok(step) => step,
        Err(_) => {
            set_last_error("panic in webui_streaming_session_resume");
            std::ptr::null_mut()
        }
    }
}

/// Emit a projected state patch for a committed updatable occurrence.
///
/// Returns allocated bytes that must be freed with [`webui_free`]. On success,
/// `out_len` receives the authoritative byte length excluding the allocation's
/// trailing NUL. On failure, `NULL` is returned and `out_len` is untouched.
/// Call [`webui_last_error`] for details.
///
/// # Safety
///
/// * `session_ptr` must be a live session handle with no concurrent operation.
/// * `patch_json` must be non-null, null-terminated UTF-8 and remain readable
///   for this call.
/// * `out_len` must be non-null and writable.
#[no_mangle]
pub unsafe extern "C" fn webui_streaming_session_update(
    session_ptr: *mut webui_streaming_session_t,
    instance_id: u32,
    patch_json: *const c_char,
    out_len: *mut usize,
) -> *mut c_char {
    clear_last_error();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if session_ptr.is_null() || patch_json.is_null() || out_len.is_null() {
            set_last_error("session_ptr, patch_json, and out_len must not be null");
            return std::ptr::null_mut();
        }
        // SAFETY: The caller guarantees exclusive access to the live session.
        let context = unsafe { &mut *(session_ptr as *mut StreamingSessionContext) };
        // SAFETY: The caller guarantees the string is readable and terminated.
        let Some(patch) = (unsafe { streaming_json_arg(patch_json, "patch_json") }) else {
            return std::ptr::null_mut();
        };
        let bytes = match context
            .session
            .update(BoundaryInstanceId::from_raw(instance_id), &patch)
        {
            Ok(bytes) => bytes,
            Err(error) => {
                set_last_error(error.to_string());
                return std::ptr::null_mut();
            }
        };
        let length = bytes.len();
        match CString::new(bytes) {
            Ok(chunk) => {
                // SAFETY: The caller guarantees `out_len` is writable.
                unsafe { *out_len = length };
                chunk.into_raw()
            }
            Err(error) => {
                set_last_error(format!(
                    "streaming update contains an interior NUL byte: {error}"
                ));
                std::ptr::null_mut()
            }
        }
    })) {
        Ok(bytes) => bytes,
        Err(_) => {
            set_last_error("panic in webui_streaming_session_update");
            std::ptr::null_mut()
        }
    }
}

/// Release an owned streaming step.
///
/// Destroying a step invalidates its byte pointer and all descriptor string
/// pointers previously returned by step accessors.
///
/// # Safety
///
/// `step_ptr` must be a pointer returned by
/// [`webui_streaming_session_start`] or [`webui_streaming_session_resume`], or
/// `NULL` for a no-op. A non-null pointer must not be used after this call.
#[no_mangle]
pub unsafe extern "C" fn webui_streaming_step_destroy(step_ptr: *mut webui_streaming_step_t) {
    clear_last_error();
    if step_ptr.is_null() {
        return;
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: The caller guarantees this is one live owned step.
        drop(unsafe { Box::from_raw(step_ptr as *mut StreamingStepContext) });
    }));
    if result.is_err() {
        set_last_error("panic in webui_streaming_step_destroy");
    }
}

/// Borrow the bytes produced by this step and write their length to `out_len`.
///
/// The returned pointer is borrowed from `step_ptr`, is not NUL-terminated,
/// and remains valid only until [`webui_streaming_step_destroy`]. It may be
/// read for exactly `out_len` bytes. Returns `NULL` on error.
///
/// # Safety
///
/// * `step_ptr` must be a live step handle with no concurrent destroy.
/// * `out_len` must be non-null and writable.
#[no_mangle]
pub unsafe extern "C" fn webui_streaming_step_bytes(
    step_ptr: *const webui_streaming_step_t,
    out_len: *mut usize,
) -> *const u8 {
    clear_last_error();
    match std::panic::catch_unwind(|| {
        if step_ptr.is_null() || out_len.is_null() {
            set_last_error("step_ptr and out_len must not be null");
            return std::ptr::null();
        }
        // SAFETY: The caller guarantees a live step handle.
        let context = unsafe { &*(step_ptr as *const StreamingStepContext) };
        // SAFETY: The caller guarantees `out_len` is writable.
        unsafe { *out_len = context.step.bytes.len() };
        context.step.bytes.as_ptr()
    }) {
        Ok(bytes) => bytes,
        Err(_) => {
            set_last_error("panic in webui_streaming_step_bytes");
            std::ptr::null()
        }
    }
}

/// Observe whether this step emitted the terminal record.
///
/// A valid non-terminal step returns `false` with no last error. A null handle
/// returns `false` and sets [`webui_last_error`].
///
/// # Safety
///
/// `step_ptr` must be a live step handle with no concurrent destroy.
#[no_mangle]
pub unsafe extern "C" fn webui_streaming_step_done(
    step_ptr: *const webui_streaming_step_t,
) -> bool {
    clear_last_error();
    match std::panic::catch_unwind(|| {
        if step_ptr.is_null() {
            set_last_error("step_ptr is null");
            return false;
        }
        // SAFETY: The caller guarantees a live step handle.
        unsafe { (*(step_ptr as *const StreamingStepContext)).step.done }
    }) {
        Ok(done) => done,
        Err(_) => {
            set_last_error("panic in webui_streaming_step_done");
            false
        }
    }
}

/// Observe whether this step carries a pending boundary descriptor.
///
/// A valid boundary-free step returns `false` with no last error. A null handle
/// returns `false` and sets [`webui_last_error`].
///
/// # Safety
///
/// `step_ptr` must be a live step handle with no concurrent destroy.
#[no_mangle]
pub unsafe extern "C" fn webui_streaming_step_has_boundary(
    step_ptr: *const webui_streaming_step_t,
) -> bool {
    clear_last_error();
    match std::panic::catch_unwind(|| {
        if step_ptr.is_null() {
            set_last_error("step_ptr is null");
            return false;
        }
        // SAFETY: The caller guarantees a live step handle.
        unsafe {
            (*(step_ptr as *const StreamingStepContext))
                .step
                .boundary
                .is_some()
        }
    }) {
        Ok(has_boundary) => has_boundary,
        Err(_) => {
            set_last_error("panic in webui_streaming_step_has_boundary");
            false
        }
    }
}

/// Read the pending boundary's response-local instance ID.
///
/// Returns `false` and leaves `out_instance_id` untouched when the step has no
/// boundary or an argument is invalid.
///
/// # Safety
///
/// * `step_ptr` must be a live step handle with no concurrent destroy.
/// * `out_instance_id` must be non-null and writable.
#[no_mangle]
pub unsafe extern "C" fn webui_streaming_step_boundary_instance_id(
    step_ptr: *const webui_streaming_step_t,
    out_instance_id: *mut u32,
) -> bool {
    clear_last_error();
    match std::panic::catch_unwind(|| {
        if step_ptr.is_null() || out_instance_id.is_null() {
            set_last_error("step_ptr and out_instance_id must not be null");
            return false;
        }
        // SAFETY: The caller guarantees a live step handle.
        let context = unsafe { &*(step_ptr as *const StreamingStepContext) };
        let Some(boundary) = streaming_step_boundary(context) else {
            return false;
        };
        // SAFETY: The caller guarantees `out_instance_id` is writable.
        unsafe { *out_instance_id = boundary.instance_id.raw() };
        true
    }) {
        Ok(ok) => ok,
        Err(_) => {
            set_last_error("panic in webui_streaming_step_boundary_instance_id");
            false
        }
    }
}

/// Read the pending boundary's stable compiler declaration ID.
///
/// Returns `false` and leaves `out_declaration_id` untouched when the step has
/// no boundary or an argument is invalid.
///
/// # Safety
///
/// * `step_ptr` must be a live step handle with no concurrent destroy.
/// * `out_declaration_id` must be non-null and writable.
#[no_mangle]
pub unsafe extern "C" fn webui_streaming_step_boundary_declaration_id(
    step_ptr: *const webui_streaming_step_t,
    out_declaration_id: *mut u32,
) -> bool {
    clear_last_error();
    match std::panic::catch_unwind(|| {
        if step_ptr.is_null() || out_declaration_id.is_null() {
            set_last_error("step_ptr and out_declaration_id must not be null");
            return false;
        }
        // SAFETY: The caller guarantees a live step handle.
        let context = unsafe { &*(step_ptr as *const StreamingStepContext) };
        let Some(boundary) = streaming_step_boundary(context) else {
            return false;
        };
        // SAFETY: The caller guarantees `out_declaration_id` is writable.
        unsafe { *out_declaration_id = boundary.declaration_id };
        true
    }) {
        Ok(ok) => ok,
        Err(_) => {
            set_last_error("panic in webui_streaming_step_boundary_declaration_id");
            false
        }
    }
}

/// Borrow the pending boundary owner's UTF-8 bytes.
///
/// The returned string pointer is not NUL-terminated. It is borrowed from the
/// step and remains valid only until [`webui_streaming_step_destroy`]. Read it
/// for exactly the byte length written to `out_len`. Returns `NULL` on error.
///
/// # Safety
///
/// * `step_ptr` must be a live step handle with no concurrent destroy.
/// * `out_len` must be non-null and writable.
#[no_mangle]
pub unsafe extern "C" fn webui_streaming_step_boundary_owner(
    step_ptr: *const webui_streaming_step_t,
    out_len: *mut usize,
) -> *const c_char {
    clear_last_error();
    match std::panic::catch_unwind(|| {
        if step_ptr.is_null() || out_len.is_null() {
            set_last_error("step_ptr and out_len must not be null");
            return std::ptr::null();
        }
        // SAFETY: The caller guarantees a live step handle.
        let context = unsafe { &*(step_ptr as *const StreamingStepContext) };
        let Some(boundary) = streaming_step_boundary(context) else {
            return std::ptr::null();
        };
        // SAFETY: The caller guarantees `out_len` is writable.
        unsafe { *out_len = boundary.owner.len() };
        boundary.owner.as_ptr().cast()
    }) {
        Ok(owner) => owner,
        Err(_) => {
            set_last_error("panic in webui_streaming_step_boundary_owner");
            std::ptr::null()
        }
    }
}

/// Borrow the pending boundary name's UTF-8 bytes.
///
/// The returned string pointer is not NUL-terminated. It is borrowed from the
/// step and remains valid only until [`webui_streaming_step_destroy`]. Read it
/// for exactly the byte length written to `out_len`. Returns `NULL` on error.
///
/// # Safety
///
/// * `step_ptr` must be a live step handle with no concurrent destroy.
/// * `out_len` must be non-null and writable.
#[no_mangle]
pub unsafe extern "C" fn webui_streaming_step_boundary_name(
    step_ptr: *const webui_streaming_step_t,
    out_len: *mut usize,
) -> *const c_char {
    clear_last_error();
    match std::panic::catch_unwind(|| {
        if step_ptr.is_null() || out_len.is_null() {
            set_last_error("step_ptr and out_len must not be null");
            return std::ptr::null();
        }
        // SAFETY: The caller guarantees a live step handle.
        let context = unsafe { &*(step_ptr as *const StreamingStepContext) };
        let Some(boundary) = streaming_step_boundary(context) else {
            return std::ptr::null();
        };
        // SAFETY: The caller guarantees `out_len` is writable.
        unsafe { *out_len = boundary.name.len() };
        boundary.name.as_ptr().cast()
    }) {
        Ok(name) => name,
        Err(_) => {
            set_last_error("panic in webui_streaming_step_boundary_name");
            std::ptr::null()
        }
    }
}

/// Read the pending boundary key discriminator.
///
/// On success, writes exactly one of [`WEBUI_BOUNDARY_KEY_NONE`],
/// [`WEBUI_BOUNDARY_KEY_STRING`], or [`WEBUI_BOUNDARY_KEY_NUMBER`]. Returns
/// `false` and leaves `out_key_type` untouched on error.
///
/// # Safety
///
/// * `step_ptr` must be a live step handle with no concurrent destroy.
/// * `out_key_type` must be non-null and writable.
#[no_mangle]
pub unsafe extern "C" fn webui_streaming_step_boundary_key_type(
    step_ptr: *const webui_streaming_step_t,
    out_key_type: *mut webui_boundary_key_type_t,
) -> bool {
    clear_last_error();
    match std::panic::catch_unwind(|| {
        if step_ptr.is_null() || out_key_type.is_null() {
            set_last_error("step_ptr and out_key_type must not be null");
            return false;
        }
        // SAFETY: The caller guarantees a live step handle.
        let context = unsafe { &*(step_ptr as *const StreamingStepContext) };
        let Some(boundary) = streaming_step_boundary(context) else {
            return false;
        };
        let key_type = match boundary.key {
            None => WEBUI_BOUNDARY_KEY_NONE,
            Some(BoundaryKey::String(_)) => WEBUI_BOUNDARY_KEY_STRING,
            Some(BoundaryKey::Number(_)) => WEBUI_BOUNDARY_KEY_NUMBER,
        };
        // SAFETY: The caller guarantees `out_key_type` is writable.
        unsafe { *out_key_type = key_type };
        true
    }) {
        Ok(ok) => ok,
        Err(_) => {
            set_last_error("panic in webui_streaming_step_boundary_key_type");
            false
        }
    }
}

/// Borrow the pending boundary's string key as UTF-8 bytes.
///
/// The returned string pointer is not NUL-terminated. It is borrowed from the
/// step and remains valid only until [`webui_streaming_step_destroy`]. Read it
/// for exactly the byte length written to `out_len`. A non-string key returns
/// `NULL`, leaves `out_len` untouched, and sets [`webui_last_error`].
///
/// # Safety
///
/// * `step_ptr` must be a live step handle with no concurrent destroy.
/// * `out_len` must be non-null and writable.
#[no_mangle]
pub unsafe extern "C" fn webui_streaming_step_boundary_key_string(
    step_ptr: *const webui_streaming_step_t,
    out_len: *mut usize,
) -> *const c_char {
    clear_last_error();
    match std::panic::catch_unwind(|| {
        if step_ptr.is_null() || out_len.is_null() {
            set_last_error("step_ptr and out_len must not be null");
            return std::ptr::null();
        }
        // SAFETY: The caller guarantees a live step handle.
        let context = unsafe { &*(step_ptr as *const StreamingStepContext) };
        let Some(boundary) = streaming_step_boundary(context) else {
            return std::ptr::null();
        };
        let Some(BoundaryKey::String(value)) = boundary.key.as_ref() else {
            set_last_error("pending boundary key is not a string");
            return std::ptr::null();
        };
        // SAFETY: The caller guarantees `out_len` is writable.
        unsafe { *out_len = value.len() };
        value.as_ptr().cast()
    }) {
        Ok(value) => value,
        Err(_) => {
            set_last_error("panic in webui_streaming_step_boundary_key_string");
            std::ptr::null()
        }
    }
}

/// Read the pending boundary's numeric key.
///
/// Returns `false`, leaves `out_value` untouched, and sets
/// [`webui_last_error`] when the key is absent or is a string.
///
/// # Safety
///
/// * `step_ptr` must be a live step handle with no concurrent destroy.
/// * `out_value` must be non-null and writable.
#[no_mangle]
pub unsafe extern "C" fn webui_streaming_step_boundary_key_number(
    step_ptr: *const webui_streaming_step_t,
    out_value: *mut f64,
) -> bool {
    clear_last_error();
    match std::panic::catch_unwind(|| {
        if step_ptr.is_null() || out_value.is_null() {
            set_last_error("step_ptr and out_value must not be null");
            return false;
        }
        // SAFETY: The caller guarantees a live step handle.
        let context = unsafe { &*(step_ptr as *const StreamingStepContext) };
        let Some(boundary) = streaming_step_boundary(context) else {
            return false;
        };
        let Some(BoundaryKey::Number(value)) = boundary.key.as_ref() else {
            set_last_error("pending boundary key is not a number");
            return false;
        };
        let Some(value) = value.as_f64() else {
            set_last_error("pending boundary key cannot be represented as a finite number");
            return false;
        };
        // SAFETY: The caller guarantees `out_value` is writable.
        unsafe { *out_value = value };
        true
    }) {
        Ok(ok) => ok,
        Err(_) => {
            set_last_error("panic in webui_streaming_step_boundary_key_number");
            false
        }
    }
}

fn owned_streaming_step(step: StreamStep) -> *mut webui_streaming_step_t {
    Box::into_raw(Box::new(StreamingStepContext { step })) as *mut webui_streaming_step_t
}

fn streaming_step_boundary(context: &StreamingStepContext) -> Option<&BoundaryDescriptor> {
    match context.step.boundary.as_ref() {
        Some(boundary) => Some(boundary),
        None => {
            set_last_error("streaming step has no pending boundary");
            None
        }
    }
}

fn streaming_boundary_mode(mode: webui_boundary_mode_t) -> Option<BoundaryMode> {
    match mode {
        WEBUI_BOUNDARY_MODE_FINAL => Some(BoundaryMode::Final),
        WEBUI_BOUNDARY_MODE_UPDATABLE => Some(BoundaryMode::Updatable),
        _ => {
            set_last_error(format!(
                "invalid boundary mode {mode}; expected WEBUI_BOUNDARY_MODE_FINAL (0) \
                 or WEBUI_BOUNDARY_MODE_UPDATABLE (1)"
            ));
            None
        }
    }
}

/// Parse one required JSON string argument.
///
/// # Safety
///
/// `value` must be non-null, null-terminated, and readable for this call.
unsafe fn streaming_json_arg(value: *const c_char, name: &str) -> Option<Value> {
    // SAFETY: The caller guarantees the string pointer is valid and terminated.
    let text = unsafe { utf8_arg(value, name) }?;
    match serde_json::from_str(text) {
        Ok(value) => Some(value),
        Err(error) => {
            set_last_error(format!("failed to parse {name}: {error}"));
            None
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
