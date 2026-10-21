// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{clear_last_error, set_last_error, utf8_arg, webui_protocol_t, ProtocolContext};
use std::ffi::{c_char, CString};
use std::panic::{catch_unwind, UnwindSafe};
use webui_handler::{DeferredOptions, DeferredRequest};

struct Arguments<'a> {
    protocol: &'a ProtocolContext,
    owner: &'a str,
    id: &'a str,
    options: DeferredOptions,
}

impl Arguments<'_> {
    fn request(&self) -> DeferredRequest<'_> {
        DeferredRequest::new(self.owner, self.id).with_options(&self.options)
    }
}

// SAFETY: Non-null inputs must satisfy the corresponding exported function's contract.
unsafe fn arguments<'a>(
    protocol: *const webui_protocol_t,
    owner: *const c_char,
    id: *const c_char,
    options: *const c_char,
) -> Option<Arguments<'a>> {
    if protocol.is_null() || owner.is_null() || id.is_null() {
        set_last_error("protocol, owner, and deferred id must not be null");
        return None;
    }
    // SAFETY: The caller supplies a live protocol and readable terminated strings.
    let protocol = unsafe { &*protocol.cast::<ProtocolContext>() };
    // SAFETY: Null pointers were rejected above; validity is the caller's contract.
    let owner = unsafe { utf8_arg(owner, "owner") }?;
    // SAFETY: Null pointers were rejected above; validity is the caller's contract.
    let id = unsafe { utf8_arg(id, "id") }?;
    let options = if options.is_null() {
        DeferredOptions::default()
    } else {
        // SAFETY: A present options pointer must be readable and terminated.
        let text = unsafe { utf8_arg(options, "deferred options") }?;
        match serde_json::from_str(text) {
            Ok(options) => options,
            Err(error) => {
                set_last_error(format!("invalid deferred options JSON: {error}"));
                return None;
            }
        }
    };
    Some(Arguments {
        protocol,
        owner,
        id,
        options,
    })
}

fn guard(name: &str, operation: impl FnOnce() -> *mut c_char + UnwindSafe) -> *mut c_char {
    clear_last_error();
    match catch_unwind(operation) {
        Ok(value) => value,
        Err(_) => {
            set_last_error(format!("panic in {name}"));
            std::ptr::null_mut()
        }
    }
}

fn string_result(result: webui_handler::Result<String>) -> *mut c_char {
    match result {
        Ok(json) => match CString::new(json) {
            Ok(value) => value.into_raw(),
            Err(error) => {
                set_last_error(format!("deferred output contains an interior NUL: {error}"));
                std::ptr::null_mut()
            }
        },
        Err(error) => {
            set_last_error(error.to_string());
            std::ptr::null_mut()
        }
    }
}

/// Render resource JSON for an owner-scoped deferred body. Free it with `webui_free`.
///
/// # Safety
///
/// `protocol` must be a live handle from `webui_protocol_create`. `owner` and `id`
/// must be non-null terminated UTF-8 strings. `options_json` may be null for
/// defaults; otherwise it must be readable, terminated UTF-8 JSON.
#[no_mangle]
pub unsafe extern "C" fn webui_protocol_render_deferred_resources(
    protocol: *const webui_protocol_t,
    owner: *const c_char,
    id: *const c_char,
    options_json: *const c_char,
) -> *mut c_char {
    guard("webui_protocol_render_deferred_resources", || {
        // SAFETY: Forwarded unchanged from this function's documented contract.
        let Some(args) = (unsafe { arguments(protocol, owner, id, options_json) }) else {
            return std::ptr::null_mut();
        };
        string_result(
            args.protocol
                .protocol
                .render_deferred_resources(args.request()),
        )
    })
}

/// Render complete or terminal JSON with fresh scoped state for a deferred body.
///
/// # Safety
///
/// `protocol` must be a live protocol handle. `state_json`, `owner`, and `id`
/// must be non-null, readable, terminated UTF-8. `options_json` may be null.
/// Release a successful returned string with `webui_free`.
#[no_mangle]
pub unsafe extern "C" fn webui_protocol_render_deferred_partial(
    protocol: *const webui_protocol_t,
    state_json: *const c_char,
    owner: *const c_char,
    id: *const c_char,
    options_json: *const c_char,
) -> *mut c_char {
    guard("webui_protocol_render_deferred_partial", || {
        if state_json.is_null() {
            set_last_error("state_json must not be null");
            return std::ptr::null_mut();
        }
        // SAFETY: Forwarded unchanged from this function's documented contract.
        let Some(args) = (unsafe { arguments(protocol, owner, id, options_json) }) else {
            return std::ptr::null_mut();
        };
        // SAFETY: Null was rejected; the caller guarantees a readable C string.
        let Some(state) = (unsafe { utf8_arg(state_json, "state_json") }) else {
            return std::ptr::null_mut();
        };
        string_result(
            args.protocol
                .protocol
                .render_deferred_partial_json(state, args.request()),
        )
    })
}

/// Prepare resource JSON and a small inventory-options JSON acknowledgement.
///
/// # Safety
///
/// The protocol/string arguments follow `webui_protocol_render_deferred_resources`.
/// `updated_options_json` must point to a writable pointer slot and must not be
/// null. On success free both returned strings with `webui_free`; on failure the
/// output slot is null.
#[no_mangle]
pub unsafe extern "C" fn webui_protocol_prepare_deferred_resources(
    protocol: *const webui_protocol_t,
    owner: *const c_char,
    id: *const c_char,
    options_json: *const c_char,
    updated_options_json: *mut *mut c_char,
) -> *mut c_char {
    guard("webui_protocol_prepare_deferred_resources", || {
        if updated_options_json.is_null() {
            set_last_error("updated_options_json must point to a writable pointer slot");
            return std::ptr::null_mut();
        }
        // SAFETY: The caller guarantees this output slot is writable.
        unsafe {
            *updated_options_json = std::ptr::null_mut();
        }
        // SAFETY: Forwarded unchanged from this function's documented contract.
        let Some(args) = (unsafe { arguments(protocol, owner, id, options_json) }) else {
            return std::ptr::null_mut();
        };
        let prepared = match args
            .protocol
            .protocol
            .prepare_deferred_resources(args.request())
        {
            Ok(prepared) => prepared,
            Err(error) => {
                set_last_error(error.to_string());
                return std::ptr::null_mut();
            }
        };
        let options = match serde_json::to_string(&prepared.options) {
            Ok(options) => options,
            Err(error) => {
                set_last_error(error.to_string());
                return std::ptr::null_mut();
            }
        };
        match (CString::new(prepared.json), CString::new(options)) {
            (Ok(json), Ok(options)) => {
                // SAFETY: The output slot is valid; ownership of both strings transfers.
                unsafe {
                    *updated_options_json = options.into_raw();
                }
                json.into_raw()
            }
            (Err(error), _) | (_, Err(error)) => {
                set_last_error(format!("deferred output contains an interior NUL: {error}"));
                std::ptr::null_mut()
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::ffi::CStr;
    use webui_protocol::{ClientFragmentData, FragmentList, StateProjectionMode, WebUIProtocol};

    fn protocol() -> *mut webui_protocol_t {
        let id = "app-shell::defer:settings";
        let mut document =
            WebUIProtocol::new(HashMap::from([(id.into(), FragmentList::default())]));
        document.build_id = "fixture".into();
        document.client_fragments.insert(
            id.into(),
            ClientFragmentData {
                owner: "app-shell".into(),
                name: "settings".into(),
                entry: true,
                template_json: r#"{"h":"<p>Inline</p>","v":3}"#.into(),
                state_mode: Some(StateProjectionMode::Keys as i32),
                state_keys: vec!["message".into()],
                ..Default::default()
            },
        );
        let bytes = document.to_protobuf().unwrap();
        // SAFETY: The encoded protocol byte slice remains valid for this call.
        let handle = unsafe { crate::webui_protocol_create(bytes.as_ptr(), bytes.len()) };
        assert!(!handle.is_null());
        handle
    }

    unsafe fn take(pointer: *mut c_char) -> String {
        assert!(!pointer.is_null());
        // SAFETY: The test passes a successful, owned FFI string result.
        let value = unsafe { CStr::from_ptr(pointer) }
            .to_str()
            .unwrap()
            .to_owned();
        // SAFETY: This allocation is released exactly once through the matching API.
        unsafe {
            crate::webui_free(pointer);
        }
        value
    }

    #[test]
    fn c_abi_prepares_resources_and_renders_fresh_state_with_acknowledgement() {
        let protocol = protocol();
        let mut options = std::ptr::null_mut();
        // SAFETY: The handle is live, C literals are terminated, and the output slot is writable.
        let metadata = unsafe {
            webui_protocol_prepare_deferred_resources(
                protocol,
                c"app-shell".as_ptr(),
                c"settings".as_ptr(),
                std::ptr::null(),
                &mut options,
            )
        };
        // SAFETY: Both pointers are successful owned results.
        let metadata = unsafe { take(metadata) };
        let json: serde_json::Value = serde_json::from_str(&metadata).unwrap();
        assert!(json.get("state").is_none());
        // SAFETY: Inputs and the acknowledgement remain live throughout the call.
        let state = unsafe {
            webui_protocol_render_deferred_partial(
                protocol,
                c"{\"message\":\"fresh\",\"unused\":1}".as_ptr(),
                c"app-shell".as_ptr(),
                c"settings".as_ptr(),
                options,
            )
        };
        // SAFETY: Successful owned result.
        let state: serde_json::Value = serde_json::from_str(&unsafe { take(state) }).unwrap();
        assert!(state.get("templates").is_none());
        assert_eq!(state["state"]["message"], "fresh");
        assert!(state["state"].get("unused").is_none());
        // SAFETY: The option allocation is still live and owned by this test.
        unsafe {
            crate::webui_free(options);
        }
        // SAFETY: The same live handle and valid strings are reused.
        let plain = unsafe {
            webui_protocol_render_deferred_resources(
                protocol,
                c"app-shell".as_ptr(),
                c"settings".as_ptr(),
                std::ptr::null(),
            )
        };
        // SAFETY: Successful owned result.
        assert_eq!(unsafe { take(plain) }, metadata);
        // SAFETY: All operations are complete; destroy the live handle once.
        unsafe {
            crate::webui_protocol_destroy(protocol);
        }
    }

    #[test]
    fn c_abi_rejects_null_and_invalid_utf8_without_panicking() {
        // SAFETY: Null arguments are explicitly supported error inputs.
        let missing = unsafe {
            webui_protocol_render_deferred_resources(
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        assert!(missing.is_null());
        let protocol = protocol();
        let invalid = [255u8, 0];
        // SAFETY: The invalid UTF-8 bytes are a valid, terminated readable C string.
        let result = unsafe {
            webui_protocol_render_deferred_resources(
                protocol,
                invalid.as_ptr().cast(),
                c"settings".as_ptr(),
                std::ptr::null(),
            )
        };
        assert!(result.is_null());
        let mut options = std::ptr::dangling_mut::<c_char>();
        // SAFETY: The output slot is writable; the null protocol is an explicit error input.
        let result = unsafe {
            webui_protocol_prepare_deferred_resources(
                std::ptr::null(),
                c"app-shell".as_ptr(),
                c"settings".as_ptr(),
                std::ptr::null(),
                &mut options,
            )
        };
        assert!(result.is_null());
        assert!(options.is_null());
        // SAFETY: The live handle is destroyed exactly once.
        unsafe {
            crate::webui_protocol_destroy(protocol);
        }
    }
}
