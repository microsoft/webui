// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::ffi::{c_char, CStr};
use std::rc::Rc;

use glib::translate::ToGlibPtr;
use gtk4::{gio, glib};
use prost::Message;
use webkit6::{prelude::*, URISchemeRequest};

use super::ipc_input::{append, ReadBuffer, ReadLimits};
use crate::ipc::{IpcBridge, IpcError, IpcErrorCode, OwnedIpcHttpRequest};
use crate::{DesktopHttpMethod, DesktopProtocolResponse};

use super::ipc::GtkIpc;
use super::ipc_control::error;
use super::response::finish_ipc_request as finish_scheme_request;

struct PendingRequest(Option<URISchemeRequest>);

impl PendingRequest {
    fn finish(mut self, response: DesktopProtocolResponse) {
        if let Some(request) = self.0.take() {
            finish_scheme_request(&request, response);
        }
    }
}

impl Drop for PendingRequest {
    fn drop(&mut self) {
        if let Some(request) = self.0.take() {
            // GTK has no stop-scheme-task callback. Settle only this retained
            // old request with cancellation on navigation/teardown, never send
            // a late successful response into a replacement document.
            let mut error =
                glib::Error::new(gio::IOErrorEnum::Cancelled, "Desktop IPC request cancelled");
            request.finish_error(&mut error);
        }
    }
}

pub(super) fn handle(state: &Rc<GtkIpc>, request: &URISchemeRequest) -> bool {
    let path = match request_path(request) {
        Ok(Some(path)) if is_ipc_path(&path) => path,
        Ok(_) => return false,
        Err(error) => {
            finish_scheme_request(request, error_response(error));
            return true;
        }
    };
    let setup = prepare(state, request, path.as_str());
    let (input, limits) = match setup {
        Ok(setup) => setup,
        Err(error) => {
            finish_scheme_request(request, error_response(error));
            return true;
        }
    };
    let pending = PendingRequest(Some(request.clone()));
    let stream = request.http_body();
    let navigation = input.navigation;
    let weak = Rc::downgrade(state);
    let bridge = state.bridge.clone();
    let tasks = Rc::clone(&state.tasks.borrow());
    // Both the request and its response bytes have concrete owners across
    // every await. The driver polls only native I/O and bridge completions.
    let _ = tasks.spawn(async move {
        let result = read_body(&bridge, input, stream, limits).await;
        let Some(state) = weak.upgrade().filter(|state| state.is_current(navigation)) else {
            return;
        };
        let response = match result {
            Ok(input) => bridge.submit(input).await.unwrap_or_else(error_response),
            Err(error) => error_response(error),
        };
        if state.is_current(navigation) {
            pending.finish(response);
        }
    });
    true
}

pub(super) fn reject_unavailable(request: &URISchemeRequest) -> bool {
    match request_path(request) {
        Ok(Some(path)) if is_ipc_path(&path) => {
            finish_scheme_request(request, error_response(error(IpcErrorCode::Closed)));
            true
        }
        Err(error) => {
            finish_scheme_request(request, error_response(error));
            true
        }
        _ => false,
    }
}

fn prepare(
    state: &GtkIpc,
    request: &URISchemeRequest,
    path: &str,
) -> Result<(OwnedIpcHttpRequest, ReadLimits), IpcError> {
    if path.len() > 64 || !state.is_current(state.navigation.get()) {
        return Err(error(IpcErrorCode::NotReady));
    }
    // SAFETY: WebKit's borrowed NUL-terminated URI remains live with request.
    let uri = unsafe {
        bounded_text(
            webkit6::ffi::webkit_uri_scheme_request_get_uri(request.to_glib_none().0),
            4096,
        )
    }?
    .ok_or_else(|| error(IpcErrorCode::InvalidFrame))?;
    if !super::ipc::trusted_uri(&uri) || uri.contains('?') || uri.contains('#') {
        return Err(error(IpcErrorCode::PermissionDenied));
    }
    let headers = request
        .http_headers()
        .ok_or_else(|| error(IpcErrorCode::PermissionDenied))?;
    let token = header(&headers, c"X-WebUI-Ipc-Session", 32)?
        .filter(|token| token.len() == 32)
        .ok_or_else(|| error(IpcErrorCode::PermissionDenied))?;
    let declared = header(&headers, c"Content-Length", 20)?
        .map(|value| {
            if value.is_empty()
                || value.len() > 20
                || !value.bytes().all(|byte| byte.is_ascii_digit())
            {
                return Err(error(IpcErrorCode::InvalidFrame));
            }
            value
                .parse::<usize>()
                .map_err(|_| error(IpcErrorCode::InvalidFrame))
        })
        .transpose()?;
    let (maximum, control) = state
        .session
        .borrow()
        .as_ref()
        .map(|session| {
            (
                session.limits.max_frame_bytes,
                session.limits.max_error_text_bytes_total + 128,
            )
        })
        .ok_or_else(|| error(IpcErrorCode::NotReady))?;
    if declared.is_some_and(|length| length > maximum) {
        return Err(error(IpcErrorCode::PayloadTooLarge));
    }
    // SAFETY: WebKit's borrowed method remains live with request.
    let method = unsafe {
        bounded_text(
            webkit6::ffi::webkit_uri_scheme_request_get_http_method(request.to_glib_none().0),
            16,
        )
    }?;
    Ok((
        OwnedIpcHttpRequest {
            navigation: state.navigation.get(),
            method: method.map_or(DesktopHttpMethod::Get, |method| {
                DesktopHttpMethod::parse(&method)
            }),
            path: path.into(),
            token,
            body: Vec::new(),
            input_permit: state.bridge.reserve_input(0)?,
        },
        ReadLimits {
            maximum,
            control,
            declared,
        },
    ))
}

fn request_path(request: &URISchemeRequest) -> Result<Option<String>, IpcError> {
    const PREFIX: &[u8] = b"/_webui/ipc";
    // SAFETY: The getter returns a borrowed NUL-terminated string valid with
    // request. Each read stops at NUL; no more than the fixed prefix is read
    // before deciding whether this request belongs to the IPC adapter.
    unsafe {
        let pointer = webkit6::ffi::webkit_uri_scheme_request_get_path(request.to_glib_none().0);
        if pointer.is_null() {
            return Ok(None);
        }
        for (index, byte) in PREFIX.iter().enumerate() {
            if pointer.cast::<u8>().add(index).read() != *byte {
                return Ok(None);
            }
        }
        let next = pointer.cast::<u8>().add(PREFIX.len()).read();
        if next != 0 && next != b'/' {
            return Ok(None);
        }
        bounded_text(pointer, 64)
    }
}

fn header(
    headers: &webkit6::soup::MessageHeaders,
    name: &CStr,
    maximum: usize,
) -> Result<Option<String>, IpcError> {
    // SAFETY: libsoup owns the borrowed NUL-terminated value; headers is kept
    // alive and never mutated during this bounded read. No unbounded GString
    // allocation precedes the token/content-length checks.
    unsafe {
        bounded_text(
            webkit6::soup::ffi::soup_message_headers_get_one(
                headers.to_glib_none().0,
                name.as_ptr(),
            ),
            maximum,
        )
    }
}

// Caller supplies a valid NUL-terminated C string or null, retained throughout
// this call. The result is independently owned and bounded before allocation.
unsafe fn bounded_text(pointer: *const c_char, maximum: usize) -> Result<Option<String>, IpcError> {
    if pointer.is_null() {
        return Ok(None);
    }
    for length in 0..=maximum {
        // SAFETY: Caller guarantees a valid C string; the loop stops at its
        // first NUL, so no read crosses the string's allocation.
        if unsafe { pointer.add(length).read() } == 0 {
            // SAFETY: Exactly these length bytes were checked before the NUL.
            let bytes = unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), length) };
            return std::str::from_utf8(bytes)
                .map(|text| Some(text.to_owned()))
                .map_err(|_| error(IpcErrorCode::InvalidFrame));
        }
    }
    Err(error(IpcErrorCode::PayloadTooLarge))
}

async fn read_body(
    bridge: &IpcBridge,
    mut input: OwnedIpcHttpRequest,
    stream: Option<gio::InputStream>,
    limits: ReadLimits,
) -> Result<OwnedIpcHttpRequest, IpcError> {
    let Some(stream) = stream else {
        if limits.declared.is_some_and(|length| length != 0) {
            return Err(error(IpcErrorCode::InvalidFrame));
        }
        return Ok(input);
    };
    let mut scratch = ReadBuffer::new(bridge, limits.maximum)?;
    loop {
        let (buffer, count) = stream
            .read_future(scratch, glib::Priority::DEFAULT)
            .await
            .map_err(|_| error(IpcErrorCode::Transport))?;
        scratch = buffer;
        if count == 0 {
            break;
        }
        append(&mut input, &scratch.bytes[..count], limits)?;
        scratch.grow_for_payload(bridge, input.body.len(), limits)?;
    }
    if limits
        .declared
        .is_some_and(|length| length != input.body.len())
    {
        return Err(error(IpcErrorCode::InvalidFrame));
    }
    Ok(input)
}

fn is_ipc_path(path: &str) -> bool {
    (path == "/_webui/ipc" || path.starts_with("/_webui/ipc/"))
        && !matches!(path, "/_webui/ipc/bootstrap.js" | "/_webui/ipc/runtime.js")
}

fn error_response(error: IpcError) -> DesktopProtocolResponse {
    let status = match error.code {
        IpcErrorCode::PermissionDenied => 401,
        IpcErrorCode::PayloadTooLarge => 413,
        IpcErrorCode::Overloaded => 429,
        IpcErrorCode::Navigated | IpcErrorCode::NotReady => 409,
        IpcErrorCode::Transport | IpcErrorCode::Closed => 503,
        _ => 400,
    };
    DesktopProtocolResponse::new(
        status,
        "application/x-protobuf",
        crate::ipc::wire::WireError {
            code: error.code.as_str().into(),
            message: "Native IPC request rejected".into(),
            help: "Reload the trusted document and use the generated binary transport".into(),
            application_code: String::new(),
        }
        .encode_to_vec(),
    )
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[path = "ipc_scheme_tests.rs"]
mod tests;
