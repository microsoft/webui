// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Binary WebView2 ingress. COM streams and deferrals never leave the STA.

use std::{ffi::c_void, rc::Rc};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2Environment, ICoreWebView2HttpRequestHeaders, ICoreWebView2WebResourceRequest,
    ICoreWebView2WebResourceRequestedEventArgs,
};
use windows::core::Result as WindowsResult;

use super::{
    ipc::WindowsIpc,
    ipc_body::read_body,
    ipc_policy::error,
    protocol::{create_webview_response, read_pwstr_bounded},
};
use crate::{
    ipc::{wire::WireError, IpcError, IpcErrorCode, OwnedIpcHttpRequest},
    DesktopHttpMethod, DesktopProtocolResponse,
};

/// Run a cleanup exactly once, including when a UI-local future is cancelled.
struct OnDrop<F: FnOnce()>(Option<F>);
impl<F: FnOnce()> Drop for OnDrop<F> {
    fn drop(&mut self) {
        if let Some(complete) = self.0.take() {
            complete();
        }
    }
}

pub(super) fn error_response(code: IpcErrorCode) -> DesktopProtocolResponse {
    let status = match code {
        IpcErrorCode::PermissionDenied => 401,
        IpcErrorCode::NotReady | IpcErrorCode::Navigated => 409,
        IpcErrorCode::PayloadTooLarge => 413,
        IpcErrorCode::Overloaded => 429,
        IpcErrorCode::Closed | IpcErrorCode::Transport => 503,
        _ => 400,
    };
    let body = WireError {
        code: code.as_str().into(),
        message: code.as_str().into(),
        help: "reload the trusted document or reduce IPC work".into(),
        application_code: String::new(),
    }
    .encode_to_vec();
    DesktopProtocolResponse::new(status, "application/x-protobuf", body)
}

/// Set a safe fallback before deferring. Cancellation only completes that
/// deferral; it never invokes a success callback against a replacement document.
pub(super) fn handle(
    ipc: &Rc<WindowsIpc>,
    environment: &ICoreWebView2Environment,
    args: &ICoreWebView2WebResourceRequestedEventArgs,
    path: &str,
) -> WindowsResult<()> {
    let navigation = ipc.navigation();
    // SAFETY: All objects are owned by this STA and used only on this STA.
    unsafe {
        args.SetResponse(&create_webview_response(
            environment,
            error_response(IpcErrorCode::Overloaded),
        )?)?;
    }
    let request = match prepare(ipc, args, path) {
        Ok(request) => request,
        Err(err) => {
            // SAFETY: Immediate reply to the live resource callback.
            unsafe {
                args.SetResponse(&create_webview_response(
                    environment,
                    error_response(err.code),
                )?)?;
            }
            return Ok(());
        }
    };
    // SAFETY: GetDeferral is called inside the WebResourceRequested callback.
    let deferral = unsafe { args.GetDeferral()? };
    let completion = OnDrop(Some(move || {
        // SAFETY: NativeIpcTasks polls and drops this future on the owning STA.
        let _ = unsafe { deferral.Complete() };
    }));
    let bridge = ipc.bridge.clone();
    let weak = Rc::downgrade(ipc);
    let args = args.clone();
    let environment = environment.clone();
    // spawn rejects bounded overflow by dropping the future on this same STA,
    // completing the already-installed failure response exactly once.
    let result = ipc.spawn(async move {
        let _completion = completion;
        // Do not admit work until the bounded UI completion slot exists.
        let response = bridge
            .submit(request)
            .await
            .unwrap_or_else(|err| error_response(err.code));
        let Some(ipc) = weak.upgrade() else {
            return;
        };
        if !ipc.current(navigation) {
            return;
        }
        if let Ok(response) = create_webview_response(&environment, response) {
            // SAFETY: No await between the liveness check and this STA call.
            let _ = unsafe { args.SetResponse(&response) };
        }
    });
    if let Err(err) = result {
        // Failure was installed before acquiring the deferral. Do not invoke
        // SetResponse now: dropping the rejected future has already completed it.
        if err.code != IpcErrorCode::Overloaded {
            ipc.transport_failed(err.code);
        }
    }
    Ok(())
}

fn prepare(
    ipc: &WindowsIpc,
    args: &ICoreWebView2WebResourceRequestedEventArgs,
    path: &str,
) -> Result<OwnedIpcHttpRequest, IpcError> {
    // Resource callbacks can overtake ExecuteScript retirement delivery. The
    // native document state already knows why admission is unavailable; do not
    // turn that terminal reason back into an initial-handshake NotReady.
    if let Some(code) = ipc.document.borrow().request_error() {
        return Err(error(code));
    }
    let max_bytes = ipc.bridge.max_request_body_bytes(path)?;
    if path.len() > 128 || path.contains(['?', '#']) {
        return Err(error(IpcErrorCode::InvalidFrame));
    }
    // SAFETY: This function is synchronous within the native request callback.
    let request = unsafe { args.Request() }.map_err(|_| error(IpcErrorCode::Transport))?;
    let method = read_pwstr_bounded(16, |out| unsafe { request.Method(out) })
        .map_err(|_| error(IpcErrorCode::InvalidFrame))?;
    let method = DesktopHttpMethod::parse(&method);
    // SAFETY: Request is a live STA COM object.
    let headers = unsafe { request.Headers() }.map_err(|_| error(IpcErrorCode::Transport))?;
    if header(&headers, "Origin", 256)?.is_some_and(|origin| origin != super::APP_ORIGIN) {
        return Err(error(IpcErrorCode::PermissionDenied));
    }
    let token = header(&headers, "X-WebUI-Ipc-Session", 32)?
        .ok_or_else(|| error(IpcErrorCode::PermissionDenied))?;
    if super::ipc_policy::nonce(&token).is_none() {
        return Err(error(IpcErrorCode::PermissionDenied));
    }
    let claimed = header(&headers, "Content-Length", 20)?
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| error(IpcErrorCode::InvalidFrame))
        })
        .transpose()?;
    let mut input_permit = None;
    let body = request_body(&request, max_bytes, claimed, |bytes| {
        if let Some(permit) = &mut input_permit {
            crate::ipc::IpcInputPermit::try_grow(permit, bytes)
        } else {
            // The first reservation uses actual bytes, not zero or a claimed
            // Content-Length, so core can select emergency control credit.
            input_permit = Some(ipc.bridge.reserve_input(bytes)?);
            Ok(())
        }
    })?;
    let input_permit = match input_permit {
        Some(permit) => permit,
        None => ipc.bridge.reserve_input(0)?,
    };
    Ok(OwnedIpcHttpRequest {
        navigation: ipc.navigation(),
        method,
        path: path.into(),
        token,
        body,
        input_permit,
    })
}

fn header(
    headers: &ICoreWebView2HttpRequestHeaders,
    name: &str,
    max: usize,
) -> Result<Option<String>, IpcError> {
    let name = webview2_com::CoTaskMemPWSTR::from(name);
    let mut present = windows::core::BOOL::default();
    // SAFETY: Header object is owned by the current STA request callback.
    unsafe { headers.Contains(*name.as_ref().as_pcwstr(), &mut present) }
        .map_err(|_| error(IpcErrorCode::Transport))?;
    if !present.as_bool() {
        return Ok(None);
    }
    // SAFETY: Same live STA header object; the name buffer outlives the call.
    read_pwstr_bounded(max, |out| unsafe {
        headers.GetHeader(*name.as_ref().as_pcwstr(), out)
    })
    .map(Some)
    .map_err(|_| error(IpcErrorCode::InvalidFrame))
}

fn request_body(
    request: &ICoreWebView2WebResourceRequest,
    max_bytes: usize,
    claimed: Option<usize>,
    reserve: impl FnMut(usize) -> Result<(), IpcError>,
) -> Result<Vec<u8>, IpcError> {
    let stream = super::request::content(request).map_err(|_| error(IpcErrorCode::Transport))?;
    read_body(
        max_bytes,
        claimed,
        |buffer| {
            let Some(stream) = &stream else {
                return Ok(0);
            };
            let mut count = 0;
            let size =
                u32::try_from(buffer.len()).map_err(|_| error(IpcErrorCode::PayloadTooLarge))?;
            // SAFETY: Writable buffer is valid for `size` bytes, count is writable.
            unsafe { stream.Read(buffer.as_mut_ptr().cast::<c_void>(), size, Some(&mut count)) }
                .ok()
                .map_err(|_| error(IpcErrorCode::Transport))?;
            usize::try_from(count).map_err(|_| error(IpcErrorCode::Transport))
        },
        reserve,
    )
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::super::ipc_body::check_claim;
    use super::*;
    use crate::ipc::DEFAULT_MAX_IPC_PAYLOAD_BYTES as MAX_BODY;
    use std::cell::Cell;

    #[test]
    fn body_cap_and_reservation_are_based_on_actual_bytes() {
        for size in [0, 1, MAX_BODY, MAX_BODY + 1] {
            let mut remaining = size;
            let reserved = Cell::new(0);
            let result = read_body(
                MAX_BODY,
                None,
                |buffer| {
                    let count = remaining.min(buffer.len());
                    remaining -= count;
                    buffer[..count].fill(7);
                    Ok(count)
                },
                |bytes| {
                    reserved.set(reserved.get() + bytes);
                    Ok(())
                },
            );
            if size <= MAX_BODY {
                let body = result.unwrap();
                assert_eq!(body.len(), size);
                assert_eq!(body.capacity(), reserved.get());
            } else {
                assert_eq!(result.unwrap_err().code, IpcErrorCode::PayloadTooLarge);
            }
            assert!(reserved.get() <= MAX_BODY);
        }
    }

    #[test]
    fn failed_credit_does_not_grow_or_read_again() {
        let reads = Cell::new(0);
        let result = read_body(
            MAX_BODY,
            None,
            |buffer| {
                reads.set(reads.get() + 1);
                buffer[0] = 1;
                Ok(1)
            },
            |_| Err(error(IpcErrorCode::Overloaded)),
        );
        assert_eq!(result.unwrap_err().code, IpcErrorCode::Overloaded);
        assert_eq!(reads.get(), 1);
    }

    #[test]
    fn content_length_cannot_hide_or_inflate_actual_body() {
        assert!(check_claim(MAX_BODY, None, MAX_BODY).is_ok());
        assert!(check_claim(MAX_BODY, Some(0), 0).is_ok());
        assert!(check_claim(MAX_BODY, Some(10), 10).is_ok());
        assert_eq!(
            check_claim(MAX_BODY, Some(0), 1).unwrap_err().code,
            IpcErrorCode::InvalidFrame
        );
        assert_eq!(
            check_claim(MAX_BODY, Some(10), 9).unwrap_err().code,
            IpcErrorCode::InvalidFrame
        );
        assert_eq!(
            check_claim(MAX_BODY, Some(1), MAX_BODY + 1)
                .unwrap_err()
                .code,
            IpcErrorCode::PayloadTooLarge
        );
        assert_eq!(
            check_claim(MAX_BODY, Some(MAX_BODY + 1), 0)
                .unwrap_err()
                .code,
            IpcErrorCode::PayloadTooLarge
        );
    }

    #[test]
    fn invalid_stream_byte_count_fails_without_slicing_or_reserving() {
        let result = read_body(
            MAX_BODY,
            None,
            |buffer| Ok(buffer.len() + 1),
            |_| panic!("invalid stream must not reserve"),
        );
        assert_eq!(result.unwrap_err().code, IpcErrorCode::PayloadTooLarge);
    }

    #[test]
    fn cancelled_or_late_completion_runs_cleanup_once() {
        let completed = Cell::new(0);
        let completion = OnDrop(Some(|| completed.set(completed.get() + 1)));
        let future = async move {
            let _completion = completion;
            std::future::pending::<()>().await;
        };
        drop(future); // Navigation, close, or bounded task admission failure.
        assert_eq!(completed.get(), 1);
        let completion = OnDrop(Some(|| completed.set(completed.get() + 1)));
        drop(completion);
        assert_eq!(completed.get(), 2);
    }
}
