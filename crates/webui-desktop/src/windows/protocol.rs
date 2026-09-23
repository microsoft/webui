// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! WebView2 `WebResourceRequested` interception and the in-memory response stream.

use std::ffi::c_void;
use std::rc::Weak;
use std::sync::{Arc, Mutex};

use crate::{
    DesktopFrame, DesktopHttpMethod, DesktopProtocolRequest, DesktopProtocolResponse,
    DesktopResponseBody, DesktopRuntime, DEFAULT_MAX_REQUEST_BYTES,
};
use anyhow::{Context, Result};
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
use windows::Win32::Foundation::{E_FAIL, E_INVALIDARG, E_NOTIMPL, E_POINTER, S_FALSE, S_OK};
use windows::Win32::System::Com::{
    ISequentialStream_Impl, IStream, IStream as WinIStream, IStream_Impl, LOCKTYPE, STATFLAG,
    STATSTG, STGC, STGTY_STREAM, STREAM_SEEK, STREAM_SEEK_CUR, STREAM_SEEK_END, STREAM_SEEK_SET,
};

#[cfg(feature = "application-ipc")]
use super::ipc::WindowsIpc;
use super::{APP_ORIGIN, APP_REQUEST_FILTER};

/// Serve every app-origin request from the desktop runtime.
pub(super) fn register_runtime_handler(
    environment: &ICoreWebView2Environment,
    webview: &ICoreWebView2,
    frame: &DesktopFrame,
    tasks: Weak<super::tasks::ApplicationTasks>,
    #[cfg(feature = "application-ipc")] ipc: Weak<WindowsIpc>,
) -> Result<ICoreWebView2WebResourceRequestedEventHandler> {
    register_web_resource_filter(webview)?;
    let environment = environment.clone();
    let runtime = Arc::clone(&frame.runtime);
    let executor = Arc::clone(&frame.executor);
    let handler = WebResourceRequestedEventHandler::create(Box::new(move |_sender, args| {
        if let Some(args) = args {
            #[cfg(feature = "application-ipc")]
            {
                // Typed IPC has its own authenticated dispatch.
                // SAFETY: Request is retained by these live event args on this STA.
                let request = unsafe { args.Request()? };
                let uri = read_pwstr_bounded(8192, |out| unsafe { request.Uri(out) })?;
                if let Some(path) = super::ipc_policy::reserved_path(&uri) {
                    if let Some(ipc) = ipc.upgrade() {
                        super::ipc_http::handle(&ipc, &environment, &args, path)?;
                    } else {
                        let response = create_webview_response(
                            &environment,
                            super::ipc_http::error_response(crate::ipc::IpcErrorCode::Closed),
                        )?;
                        // SAFETY: Live args, synchronous response on its owning STA.
                        unsafe {
                            args.SetResponse(&response)?;
                        }
                    }
                    return Ok(());
                }
            }
            if let Some(tasks) = tasks.upgrade() {
                handle_web_resource_request(&environment, &runtime, &executor, &tasks, &args)?;
            }
        }
        Ok(())
    }));
    let mut token = 0_i64;
    // SAFETY: `webview` is a live COM interface and the handler is returned to
    // the caller, which keeps it alive for the lifetime of the window.
    unsafe { webview.add_WebResourceRequested(&handler, &mut token)? };
    Ok(handler)
}

/// Require the complete canonical app authority, not a host prefix.
pub(super) fn app_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix(APP_ORIGIN) else {
        return false;
    };
    rest.is_empty() || rest.starts_with(['/', '?', '#'])
}

/// Require native interception for documents, frames, and workers.
fn register_web_resource_filter(webview: &ICoreWebView2) -> Result<()> {
    // SDK 1.0.2365.46 introduced this interface (Runtime 122.0.2365.46).
    // The deprecated base filter misses worker and cross-origin frame requests.
    let webview = webview.cast::<ICoreWebView2_22>().context(
        "WebUI requires WebView2 Runtime 122.0.2365.46 or later for native resource interception; update the Microsoft Edge WebView2 Runtime",
    )?;
    // SAFETY: The interface is live and the filter is static. Registration
    // completes before startup navigation; there is no JavaScript fallback.
    unsafe {
        webview
            .AddWebResourceRequestedFilterWithRequestSourceKinds(
                APP_REQUEST_FILTER,
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
                COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
            )
            .context("Failed to register native WebUI resource interception")?;
    }
    Ok(())
}

/// Answer one intercepted request with a runtime response.
fn handle_web_resource_request(
    environment: &ICoreWebView2Environment,
    runtime: &Arc<DesktopRuntime>,
    executor: &crate::execution::ApplicationExecutor,
    tasks: &super::tasks::ApplicationTasks,
    args: &ICoreWebView2WebResourceRequestedEventArgs,
) -> WindowsResult<()> {
    // SAFETY: WebView2 keeps `args` and every interface reached through it
    // alive for the duration of this callback, and all strings are copied out
    // before it returns.
    unsafe {
        let request = args.Request()?;
        let uri = read_pwstr_bounded(8192, |out| request.Uri(out))?;
        if !app_url(&uri) {
            return Ok(());
        }
        let method = read_pwstr_bounded(32, |out| request.Method(out))?;
        let head = method.eq_ignore_ascii_case("HEAD");
        let method = DesktopHttpMethod::parse(&method);
        let headers = request.Headers()?;
        let accept = read_header(&headers, "Accept").unwrap_or_default();
        let body = read_request_body(&request)?;
        let path = webui_path_from_uri(&uri);
        let runtime = Arc::clone(runtime);
        let work = executor.submit(move || {
            runtime
                .handle_request(&DesktopProtocolRequest {
                    method,
                    path: &path,
                    body: &body,
                    wants_json: accept.contains("json") || accept.contains("ndjson"),
                })
                .unwrap_or_else(|err| DesktopProtocolResponse::text(500, err.chain_message()))
        });
        let Ok(work) = work else {
            args.SetResponse(&create_webview_response(
                environment,
                response_for_head(
                    DesktopProtocolResponse::text(
                        503,
                        "Desktop application executor is unavailable",
                    ),
                    head,
                ),
            )?)?;
            return Ok(());
        };
        let deferral = ResponseDeferral(args.GetDeferral()?);
        let environment = environment.clone();
        let args = args.clone();
        tasks
            .tasks
            .spawn(async move {
                let _deferral = deferral;
                let response = work.await.unwrap_or_else(|_| {
                    DesktopProtocolResponse::text(503, "Desktop application executor closed")
                });
                match create_webview_response(&environment, response_for_head(response, head))
                    .and_then(|response| args.SetResponse(&response))
                {
                    Ok(()) => {}
                    Err(error) => eprintln!("WebUI: native response delivery failed: {error}"),
                }
            })
            .map_err(|_| WindowsError::from(E_FAIL))?;
    }

    struct ResponseDeferral(webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Deferral);
    impl Drop for ResponseDeferral {
        fn drop(&mut self) {
            // SAFETY: Deferrals are retained and completed only on their owning STA.
            let _ = unsafe { self.0.Complete() };
        }
    }
    Ok(())
}

fn response_for_head(mut response: DesktopProtocolResponse, head: bool) -> DesktopProtocolResponse {
    // WebView2 exposes intercepted response streams verbatim, unlike an HTTP
    // server's HEAD framing. Preserve status and MIME metadata but never hand
    // the browser a body, including handler errors and executor overload.
    if head {
        response.body = Vec::new().into();
    }
    response
}

/// Read one request header, returning `None` when it is absent.
pub(super) fn read_header(headers: &ICoreWebView2HttpRequestHeaders, name: &str) -> Option<String> {
    let name = CoTaskMemPWSTR::from(name);
    // SAFETY: `headers` is live for the callback and the name buffer outlives
    // this call; a missing header is reported as an error and mapped to `None`.
    read_pwstr_bounded(8192, |out| unsafe {
        headers.GetHeader(*name.as_ref().as_pcwstr(), out)
    })
    .ok()
}

/// Bound native UTF-16 before allocating the Rust copy. Invalid UTF-16 is an
/// explicit error, never an untrusted string conversion panic.
pub(super) fn read_pwstr_bounded(
    max_units: usize,
    read: impl FnOnce(*mut PWSTR) -> WindowsResult<()>,
) -> WindowsResult<String> {
    let mut raw = PWSTR::null();
    let result = read(&mut raw);
    let _owned = CoTaskMemPWSTR::from(raw);
    result?;
    if raw.is_null() {
        return Ok(String::new());
    }
    let mut len = 0;
    // SAFETY: WebView2 returns a NUL-terminated CoTaskMem string. Stop at its
    // first NUL or the configured bound while the allocation guard is alive.
    unsafe {
        while len < max_units && *raw.0.add(len) != 0 {
            len += 1;
        }
        if *raw.0.add(len) != 0 {
            return Err(WindowsError::from(E_INVALIDARG));
        }
        String::from_utf16(std::slice::from_raw_parts(raw.0, len))
            .map_err(|_| WindowsError::from(E_INVALIDARG))
    }
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
    match super::request::content(request)? {
        Some(stream) => read_request_stream(stream),
        None => Ok(Vec::new()),
    }
}

/// Drain a request stream, rejecting bodies over the runtime asset cap.
fn read_request_stream(stream: WinIStream) -> WindowsResult<Vec<u8>> {
    let mut out = Vec::new();
    let max = usize::try_from(DEFAULT_MAX_REQUEST_BYTES).unwrap_or(usize::MAX);
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
pub(super) fn create_webview_response(
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
    headers.push_str("\r\nCache-Control: no-store\r\nPragma: no-cache\r\n");
    headers
}

/// Map a status code to its HTTP reason phrase.
fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "OK",
    }
}

/// Read-only `IStream` over an in-memory response body.
#[implement(IStream)]
pub(super) struct MemoryStream {
    // Share the complete body, not just its bytes. COM stream clones retain the
    // outbound budget lease until the final native reader releases the buffer.
    data: Arc<StreamData>,
    position: Mutex<usize>,
}

impl MemoryStream {
    /// Wrap an owned body in a stream positioned at its start.
    pub(super) fn new(data: impl Into<crate::DesktopResponseContent>) -> Self {
        let data = match data.into() {
            crate::DesktopResponseContent::Bytes(bytes) => StreamData::Bytes(bytes),
            crate::DesktopResponseContent::File(file) => {
                let length = file.len();
                StreamData::File {
                    file: Mutex::new(file),
                    length,
                }
            }
        };
        Self {
            data: Arc::new(data),
            position: Mutex::new(0),
        }
    }
}

enum StreamData {
    Bytes(DesktopResponseBody),
    File {
        file: Mutex<crate::DesktopResponseFile>,
        length: u64,
    },
}

impl StreamData {
    fn len(&self) -> usize {
        match self {
            Self::Bytes(bytes) => bytes.len(),
            Self::File { length, .. } => usize::try_from(*length).unwrap_or(usize::MAX),
        }
    }

    fn read(&self, position: usize, out: &mut [u8]) -> std::io::Result<usize> {
        use std::io::{Read, Seek, SeekFrom};
        match self {
            Self::Bytes(bytes) => {
                out.copy_from_slice(&bytes[position..position + out.len()]);
                Ok(out.len())
            }
            Self::File { file, .. } => {
                let mut file = file
                    .lock()
                    .map_err(|_| std::io::Error::other("desktop stream lock failed"))?;
                file.seek(SeekFrom::Start(position as u64))?;
                file.read_exact(out)?;
                Ok(out.len())
            }
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
        let mut count = remaining.min(requested);
        if count != 0 && pv.is_null() {
            return E_POINTER;
        }
        if count != 0 {
            // SAFETY: `pv` was checked for null above and COM guarantees it is
            // writable for `cb` bytes; `count` is clamped to both `cb` and the
            // bytes remaining in `self.data`, and the buffers cannot overlap
            // because `self.data` is private to this stream.
            unsafe {
                let out = std::slice::from_raw_parts_mut(pv.cast::<u8>(), count);
                match self.data.read(*position, out) {
                    Ok(read) => count = read,
                    Err(_) => return E_FAIL,
                }
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
        if count < requested {
            S_FALSE
        } else {
            S_OK
        }
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
        let position = *self
            .position
            .lock()
            .map_err(|_| WindowsError::from(E_FAIL))?;
        Ok(MemoryStream {
            data: Arc::clone(&self.data),
            position: Mutex::new(position),
        }
        .into())
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Released(Arc<AtomicUsize>);
    impl Drop for Released {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn intercepted_responses_publish_the_canonical_no_store_policy() {
        assert_eq!(
            response_headers("application/x-protobuf"),
            "Content-Type: application/x-protobuf\r\nCache-Control: no-store\r\nPragma: no-cache\r\n"
        );
    }

    #[test]
    fn head_omits_success_and_error_bodies_but_preserves_metadata() {
        for status in [200, 405, 500, 503] {
            let released = Arc::new(AtomicUsize::new(0));
            let response = response_for_head(
                DesktopProtocolResponse::new(
                    status,
                    "application/octet-stream",
                    DesktopResponseBody::with_guard(vec![1, 2, 3], Released(Arc::clone(&released))),
                ),
                true,
            );
            assert_eq!(response.status, status);
            assert_eq!(response.content_type, "application/octet-stream");
            assert!(response.body.as_bytes().unwrap().is_empty());
            assert_eq!(released.load(Ordering::SeqCst), 1);
        }
        let response = response_for_head(DesktopProtocolResponse::text(200, "keep"), false);
        assert_eq!(response.body, b"keep");
        let response = response_for_head(
            DesktopProtocolResponse::new(
                200,
                "application/octet-stream",
                crate::DesktopResponseContent::File(crate::DesktopResponseFile::new(
                    tempfile::tempfile().unwrap(),
                    10,
                )),
            ),
            true,
        );
        // An empty file with a declared length would fail if HEAD materialized it.
        assert!(response.body.as_bytes().unwrap().is_empty());
    }

    #[test]
    fn native_stream_clones_keep_body_and_lease_until_final_release() {
        let released = Arc::new(AtomicUsize::new(0));
        let bytes = vec![1, 2, 3];
        let pointer = bytes.as_ptr();
        let memory = MemoryStream::new(DesktopResponseBody::with_guard(
            bytes,
            Released(Arc::clone(&released)),
        ));
        assert!(
            matches!(memory.data.as_ref(), StreamData::Bytes(body) if body.as_slice().as_ptr() == pointer)
        );
        let stream: IStream = memory.into();
        // SAFETY: Both are locally implemented, live COM streams. No WebView or
        // native window is needed to exercise their reference-counted lifetime.
        let clone = unsafe { stream.Clone() }.unwrap();
        drop(stream);
        assert_eq!(released.load(Ordering::SeqCst), 0);
        let mut bytes = [0_u8; 3];
        let mut read = 0;
        // SAFETY: The destination and count are writable for the declared sizes.
        unsafe { clone.Read(bytes.as_mut_ptr().cast(), 3, Some(&mut read)) }
            .ok()
            .unwrap();
        assert_eq!(read, 3);
        assert_eq!(bytes, [1, 2, 3]);
        drop(clone);
        assert_eq!(released.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn request_uri_maps_to_runtime_path() {
        assert_eq!(webui_path_from_uri("https://app.webui.localhost"), "/");
        assert_eq!(
            webui_path_from_uri("https://app.webui.localhost/contacts?view=all"),
            "/contacts?view=all"
        );
    }

    #[test]
    fn native_file_stream_reads_at_independent_offsets_without_materialization() {
        use std::io::{Seek, Write};
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"abcdef").unwrap();
        file.rewind().unwrap();
        let memory = MemoryStream::new(crate::DesktopResponseContent::File(
            crate::DesktopResponseFile::new(file, 6),
        ));
        let mut bytes = [0; 3];
        assert_eq!(memory.data.read(2, &mut bytes).unwrap(), 3);
        assert_eq!(&bytes, b"cde");
        assert_eq!(memory.data.read(0, &mut bytes).unwrap(), 3);
        assert_eq!(&bytes, b"abc");
    }

    #[test]
    fn stream_clone_preserves_cursor_and_reports_partial_eof() {
        let stream: IStream = MemoryStream::new(b"abc".to_vec()).into();
        let mut bytes = [0; 2];
        let mut read = 0;
        // SAFETY: Live locally implemented stream with a correctly-sized destination.
        unsafe { stream.Read(bytes.as_mut_ptr().cast(), 2, Some(&mut read)) }
            .ok()
            .unwrap();
        // SAFETY: Cloning this locally owned COM stream needs no WebView runtime.
        let cloned = unsafe { stream.Clone() }.unwrap();
        // SAFETY: Same valid two-byte destination and count storage.
        assert_eq!(
            unsafe { cloned.Read(bytes.as_mut_ptr().cast(), 2, Some(&mut read)) },
            S_FALSE
        );
        assert_eq!(read, 1);
        assert_eq!(bytes[0], b'c');
    }

    #[test]
    fn resource_authority_excludes_external_and_lookalike_hosts() {
        assert!(app_url("https://app.webui.localhost/"));
        assert!(app_url("https://app.webui.localhost/resource?query=value"));
        for uri in [
            "https://example.com/",
            "https://app.webui.localhost.example.com/",
            "https://app.webui.localhost@other.example/",
            "https://app.webui.localhost:444/",
            "http://app.webui.localhost/",
        ] {
            assert!(!app_url(uri), "{uri}");
        }
    }
}
