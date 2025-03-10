//! WebUI FFI (Foreign Function Interface) for interoperability with other languages.
//!
//! This crate provides C-compatible APIs for the WebUI handler to be used from languages
//! like C#, Node.js, etc.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use serde_json::Value;
use webui_handler::WebUIHandler;
use webui_protocol::WebUIProtocol;

#[cfg(target_os = "windows")]
use windows::core::PCSTR;

struct HandlerContext {
    handler: WebUIHandler,
}

/// Create a new WebUI handler instance.
///
/// Returns a pointer to the handler context or null on failure.
#[no_mangle]
pub extern "C" fn webui_handler_create() -> *mut c_void {
    let handler = WebUIHandler::new();
    let context = Box::new(HandlerContext { handler });
    Box::into_raw(context) as *mut c_void
}

/// Destroy a WebUI handler instance.
///
/// # Safety
///
/// The handler_ptr must be a valid pointer returned by webui_handler_create.
#[no_mangle]
pub unsafe extern "C" fn webui_handler_destroy(handler_ptr: *mut c_void) {
    if !handler_ptr.is_null() {
        let _ = Box::from_raw(handler_ptr as *mut HandlerContext);
    }
}

/// Render a WebUI protocol with data.
///
/// # Arguments
///
/// * `handler_ptr` - Pointer to a WebUI handler instance
/// * `protocol_json` - JSON string of the WebUI protocol
/// * `data_json` - JSON string of the data to render with
///
/// # Returns
///
/// A pointer to a null-terminated string containing the rendered HTML.
/// The caller is responsible for freeing this memory with webui_free_string.
///
/// # Safety
///
/// The handler_ptr must be a valid pointer returned by webui_handler_create.
/// protocol_json and data_json must be valid null-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn webui_handler_render(
    handler_ptr: *mut c_void,
    protocol_json: *const c_char,
    data_json: *const c_char,
) -> *mut c_char {
    if handler_ptr.is_null() || protocol_json.is_null() || data_json.is_null() {
        return std::ptr::null_mut();
    }

    let context = &*(handler_ptr as *const HandlerContext);
    
    // Convert C strings to Rust strings
    let protocol_str = match CStr::from_ptr(protocol_json).to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    
    let data_str = match CStr::from_ptr(data_json).to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    
    // Parse protocol and data from JSON
    let protocol = match webui_protocol::WebUIProtocol::from_json(protocol_str) {
        Ok(p) => p,
        Err(_) => return std::ptr::null_mut(),
    };
    
    let data: HashMap<String, Value> = match serde_json::from_str(data_str) {
        Ok(d) => d,
        Err(_) => return std::ptr::null_mut(),
    };
    
    // Render the protocol with data
    let result = match context.handler.render(&protocol, &data) {
        Ok(r) => r,
        Err(_) => return std::ptr::null_mut(),
    };
    
    // Convert the result to a C string
    match CString::new(result) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Free a string returned by a WebUI FFI function.
///
/// # Safety
///
/// The string_ptr must be a pointer returned by a WebUI FFI function,
/// or null (in which case this function does nothing).
#[no_mangle]
pub unsafe extern "C" fn webui_free_string(string_ptr: *mut c_char) {
    if !string_ptr.is_null() {
        let _ = CString::from_raw(string_ptr);
    }
}
