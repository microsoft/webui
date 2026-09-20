// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! WebView2 `WebResourceRequested` interception and the in-memory response stream.

use std::ffi::c_void;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use webui_desktop::{
    DesktopHttpMethod, DesktopProtocolRequest, DesktopProtocolResponse, DesktopRuntime,
    DEFAULT_MAX_ASSET_BYTES,
};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2, ICoreWebView2Environment, ICoreWebView2HttpRequestHeaders,
    ICoreWebView2WebResourceRequest, ICoreWebView2WebResourceRequestedEventArgs,
    ICoreWebView2WebResourceRequestedEventHandler, ICoreWebView2WebResourceResponse,
    ICoreWebView2_22, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
    COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
};
use webview2_com::{CoTaskMemPWSTR, WebResourceRequestedEventHandler};
use windows::core::{
    implement, Error as WindowsError, Interface, Result as WindowsResult, HRESULT, PWSTR,
};
use windows::Win32::Foundation::{E_FAIL, E_INVALIDARG, E_NOTIMPL, E_POINTER, S_OK};
use windows::Win32::System::Com::{
    ISequentialStream_Impl, IStream, IStream as WinIStream, IStream_Impl, LOCKTYPE, STATFLAG,
    STATSTG, STGC, STGTY_STREAM, STREAM_SEEK, STREAM_SEEK_CUR, STREAM_SEEK_END, STREAM_SEEK_SET,
};

use super::{APP_ORIGIN, APP_REQUEST_FILTER};

/// Serve every app-origin request from the desktop runtime.
pub(super) fn register_runtime_handler(
    environment: &ICoreWebView2Environment,
    webview: &ICoreWebView2,
    runtime: Arc<DesktopRuntime>,
) -> Result<ICoreWebView2WebResourceRequestedEventHandler> {
    register_web_resource_filter(webview)?;
    let environment = environment.clone();
    let handler = WebResourceRequestedEventHandler::create(Box::new(move |_sender, args| {
        if let Some(args) = args {
            handle_web_resource_request(&environment, &runtime, &args)?;
        }
        Ok(())
    }));
    let mut token = 0_i64;
    // SAFETY: `webview` is a live COM interface and the handler is returned to
    // the caller, which keeps it alive for the lifetime of the window.
    unsafe { webview.add_WebResourceRequested(&handler, &mut token)? };
    Ok(handler)
}

/// Subscribe to every request kind the installed runtime supports.
fn register_web_resource_filter(webview: &ICoreWebView2) -> Result<()> {
    // SAFETY: `webview` is a live COM interface and both filter constants are
    // static values valid for the duration of the call. The `ICoreWebView2_22`
    // cast fails on older runtimes, which fall back to the base filter API.
    unsafe {
        if let Ok(webview) = webview.cast::<ICoreWebView2_22>() {
            webview.AddWebResourceRequestedFilterWithRequestSourceKinds(
                APP_REQUEST_FILTER,
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
                COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
            )?;
            return Ok(());
        }

        webview.AddWebResourceRequestedFilter(
            APP_REQUEST_FILTER,
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
        )?;
    }
    Ok(())
}

/// Answer one intercepted request with a runtime response.
fn handle_web_resource_request(
    environment: &ICoreWebView2Environment,
    runtime: &DesktopRuntime,
    args: &ICoreWebView2WebResourceRequestedEventArgs,
) -> WindowsResult<()> {
    // SAFETY: WebView2 keeps `args` and every interface reached through it
    // alive for the duration of this callback, and all strings are copied out
    // before it returns.
    unsafe {
        let request = args.Request()?;
        let uri = read_pwstr(|out| request.Uri(out))?;
        let method = read_pwstr(|out| request.Method(out))?;
        let method = DesktopHttpMethod::parse(&method);
        let headers = request.Headers()?;
        let accept = read_header(&headers, "Accept").unwrap_or_default();
        let body = read_request_body(&request)?;
        let path = webui_path_from_uri(&uri);
        let desktop_request = DesktopProtocolRequest {
            method,
            path: &path,
            body: &body,
            wants_json: accept.contains("json") || accept.contains("ndjson"),
        };
        let response = runtime
            .handle_request(&desktop_request)
            .unwrap_or_else(|err| DesktopProtocolResponse::text(500, err.chain_message()));
        let response = create_webview_response(environment, response)?;
        args.SetResponse(&response)?;
    }
    Ok(())
}

/// Read one request header, returning `None` when it is absent.
fn read_header(headers: &ICoreWebView2HttpRequestHeaders, name: &str) -> Option<String> {
    let name = CoTaskMemPWSTR::from(name);
    // SAFETY: `headers` is live for the callback and the name buffer outlives
    // this call; a missing header is reported as an error and mapped to `None`.
    read_pwstr(|out| unsafe { headers.GetHeader(*name.as_ref().as_pcwstr(), out) }).ok()
}

/// Read a WebView2 `PWSTR` out-parameter into an owned `String`.
///
/// The returned buffer is freed through [`CoTaskMemPWSTR`], which matches the
/// `CoTaskMemAlloc` allocation WebView2 performs for these out-parameters.
pub(super) fn read_pwstr<F>(read: F) -> WindowsResult<String>
where
    F: FnOnce(*mut PWSTR) -> WindowsResult<()>,
{
    let mut raw = PWSTR::null();
    read(&mut raw)?;
    let value = CoTaskMemPWSTR::from(raw).to_string();
    Ok(value)
}

/// Convert an app-origin request URI into a runtime path.
pub(super) fn webui_path_from_uri(uri: &str) -> String {
    let rest = uri.strip_prefix(APP_ORIGIN).unwrap_or(uri);
    if rest.is_empty() {
        "/".to_string()
    } else {
        rest.to_string()
    }
}

/// Read the request body, treating a missing content stream as empty.
fn read_request_body(request: &ICoreWebView2WebResourceRequest) -> WindowsResult<Vec<u8>> {
    // SAFETY: The COM request object is valid for the duration of the
    // WebResourceRequested callback. A null content stream means an empty body.
    match unsafe { request.Content() } {
        Ok(stream) => read_request_stream(stream),
        Err(error) if error.code() == E_POINTER => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

/// Drain a request stream, rejecting bodies over the runtime asset cap.
fn read_request_stream(stream: WinIStream) -> WindowsResult<Vec<u8>> {
    let mut out = Vec::new();
    let max = usize::try_from(DEFAULT_MAX_ASSET_BYTES).unwrap_or(usize::MAX);
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let mut read = 0_u32;
        // SAFETY: `buffer` is valid for `buffer.len()` bytes and `read` points
        // to writable stack storage for the byte count.
        let hr = unsafe {
            stream.Read(
                buffer.as_mut_ptr().cast::<c_void>(),
                u32::try_from(buffer.len()).unwrap_or(u32::MAX),
                Some(&mut read),
            )
        };
        hr.ok()?;
        if read == 0 {
            break;
        }
        let read = usize::try_from(read).map_err(|_| WindowsError::from(E_INVALIDARG))?;
        if out.len().saturating_add(read) > max {
            return Err(WindowsError::from(E_INVALIDARG));
        }
        out.extend_from_slice(&buffer[..read]);
    }
    Ok(out)
}

/// Wrap a runtime response in a WebView2 response object.
fn create_webview_response(
    environment: &ICoreWebView2Environment,
    response: DesktopProtocolResponse,
) -> WindowsResult<ICoreWebView2WebResourceResponse> {
    let headers = response_headers(&response.content_type);
    let reason = status_reason(response.status);
    let status = i32::from(response.status);
    let stream: WinIStream = MemoryStream::new(response.body).into();
    let reason = CoTaskMemPWSTR::from(reason);
    let headers = CoTaskMemPWSTR::from(headers.as_str());
    // SAFETY: `environment` is live, the stream is a COM object owned by this
    // call, and both string buffers outlive the call.
    unsafe {
        environment.CreateWebResourceResponse(
            &stream,
            status,
            *reason.as_ref().as_pcwstr(),
            *headers.as_ref().as_pcwstr(),
        )
    }
}

/// Build the response header block served to web content.
fn response_headers(content_type: &str) -> String {
    let mut headers = String::with_capacity(content_type.len() + 96);
    headers.push_str("Content-Type: ");
    headers.push_str(content_type);
    headers
        .push_str("\r\nCache-Control: no-store, no-cache, must-revalidate\r\nPragma: no-cache\r\n");
    headers
}

/// Map a status code to its HTTP reason phrase.
fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        _ => "OK",
    }
}

/// Read-only `IStream` over an in-memory response body.
#[implement(IStream)]
struct MemoryStream {
    data: Arc<[u8]>,
    position: Mutex<usize>,
}

impl MemoryStream {
    /// Wrap an owned body in a stream positioned at its start.
    fn new(data: Vec<u8>) -> Self {
        Self {
            data: data.into(),
            position: Mutex::new(0),
        }
    }
}

impl ISequentialStream_Impl for MemoryStream_Impl {
    fn Read(&self, pv: *mut c_void, cb: u32, pcbread: *mut u32) -> HRESULT {
        let Ok(mut position) = self.position.lock() else {
            return E_FAIL;
        };
        let remaining = self.data.len().saturating_sub(*position);
        let requested = usize::try_from(cb).unwrap_or(usize::MAX);
        let count = remaining.min(requested);
        if count != 0 && pv.is_null() {
            return E_POINTER;
        }
        if count != 0 {
            // SAFETY: `pv` was checked for null above and COM guarantees it is
            // writable for `cb` bytes; `count` is clamped to both `cb` and the
            // bytes remaining in `self.data`, and the buffers cannot overlap
            // because `self.data` is private to this stream.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    self.data[*position..].as_ptr(),
                    pv.cast::<u8>(),
                    count,
                );
            }
            *position += count;
        }
        if !pcbread.is_null() {
            // SAFETY: `pcbread` was checked for null and COM guarantees it
            // points to a writable `u32` out-parameter.
            unsafe {
                *pcbread = u32::try_from(count).unwrap_or(u32::MAX);
            }
        }
        S_OK
    }

    fn Write(&self, _pv: *const c_void, _cb: u32, _pcbwritten: *mut u32) -> HRESULT {
        E_NOTIMPL
    }
}

impl IStream_Impl for MemoryStream_Impl {
    fn Seek(
        &self,
        dlibmove: i64,
        dworigin: STREAM_SEEK,
        plibnewposition: *mut u64,
    ) -> WindowsResult<()> {
        let mut position = self
            .position
            .lock()
            .map_err(|_| WindowsError::from(E_FAIL))?;
        let base = match dworigin {
            STREAM_SEEK_SET => 0_i64,
            STREAM_SEEK_CUR => {
                i64::try_from(*position).map_err(|_| WindowsError::from(E_INVALIDARG))?
            }
            STREAM_SEEK_END => {
                i64::try_from(self.data.len()).map_err(|_| WindowsError::from(E_INVALIDARG))?
            }
            _ => return Err(WindowsError::from(E_INVALIDARG)),
        };
        let next = base
            .checked_add(dlibmove)
            .ok_or_else(|| WindowsError::from(E_INVALIDARG))?;
        if next < 0 {
            return Err(WindowsError::from(E_INVALIDARG));
        }
        *position = usize::try_from(next).map_err(|_| WindowsError::from(E_INVALIDARG))?;
        if !plibnewposition.is_null() {
            // SAFETY: `plibnewposition` was checked for null and COM guarantees
            // it points to a writable `u64` out-parameter.
            unsafe {
                *plibnewposition =
                    u64::try_from(*position).map_err(|_| WindowsError::from(E_INVALIDARG))?;
            }
        }

        Ok(())
    }

    fn SetSize(&self, _libnewsize: u64) -> WindowsResult<()> {
        Err(WindowsError::from(E_NOTIMPL))
    }

    fn CopyTo(
        &self,
        _pstm: windows::core::Ref<IStream>,
        _cb: u64,
        _pcbread: *mut u64,
        _pcbwritten: *mut u64,
    ) -> WindowsResult<()> {
        Err(WindowsError::from(E_NOTIMPL))
    }

    fn Commit(&self, _grfcommitflags: &STGC) -> WindowsResult<()> {
        Ok(())
    }

    fn Revert(&self) -> WindowsResult<()> {
        Err(WindowsError::from(E_NOTIMPL))
    }

    fn LockRegion(&self, _liboffset: u64, _cb: u64, _dwlocktype: &LOCKTYPE) -> WindowsResult<()> {
        Err(WindowsError::from(E_NOTIMPL))
    }

    fn UnlockRegion(&self, _liboffset: u64, _cb: u64, _dwlocktype: u32) -> WindowsResult<()> {
        Err(WindowsError::from(E_NOTIMPL))
    }

    fn Stat(&self, pstatstg: *mut STATSTG, _grfstatflag: &STATFLAG) -> WindowsResult<()> {
        if pstatstg.is_null() {
            return Err(WindowsError::from(E_INVALIDARG));
        }
        // SAFETY: `pstatstg` was checked for null and COM guarantees it points
        // to a writable `STATSTG`, which is overwritten in full here.
        unsafe {
            (*pstatstg) = STATSTG::default();
            (*pstatstg).r#type = STGTY_STREAM.0.cast_unsigned();
            (*pstatstg).cbSize =
                u64::try_from(self.data.len()).map_err(|_| WindowsError::from(E_INVALIDARG))?;
        }
        Ok(())
    }

    fn Clone(&self) -> WindowsResult<IStream> {
        Ok(MemoryStream {
            data: Arc::clone(&self.data),
            position: Mutex::new(0),
        }
        .into())
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn request_uri_maps_to_runtime_path() {
        assert_eq!(webui_path_from_uri("https://app.webui.localhost"), "/");
        assert_eq!(
            webui_path_from_uri("https://app.webui.localhost/contacts?view=all"),
            "/contacts?view=all"
        );
    }
}
