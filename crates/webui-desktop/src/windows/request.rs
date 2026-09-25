// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Preserve WebView2's nullable request-body contract at the COM boundary.

use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2WebResourceRequest;
use windows::{
    core::{Interface, Result},
    Win32::System::Com::IStream,
};

/// `get_Content` succeeds with a null stream for bodyless requests. The
/// generated `Content()` getter instead returns `Err(Error::empty())` for that
/// successful null (not E_POINTER). Read the ABI's nullable result explicitly,
/// without treating real COM failures as empty request bodies.
pub(super) fn content(request: &ICoreWebView2WebResourceRequest) -> Result<Option<IStream>> {
    let mut raw = std::ptr::null_mut();
    // SAFETY: The request is live on its owning STA, and `raw` is a writable
    // interface out-parameter initialized to null. A successful non-null result
    // transfers one COM reference, which is adopted exactly once below.
    unsafe {
        (request.vtable().Content)(request.as_raw(), &mut raw).ok()?;
        Ok(if raw.is_null() {
            None
        } else {
            Some(IStream::from_raw(raw))
        })
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2HttpRequestHeaders, ICoreWebView2WebResourceRequest_Impl,
    };
    use windows::{
        core::{implement, Error, Ref, PCWSTR, PWSTR},
        Win32::Foundation::{E_FAIL, E_NOTIMPL, E_POINTER, S_OK},
    };

    #[implement(ICoreWebView2WebResourceRequest)]
    struct Request(Result<IStream>);

    impl ICoreWebView2WebResourceRequest_Impl for Request_Impl {
        fn Uri(&self, _: *mut PWSTR) -> Result<()> {
            Err(E_NOTIMPL.into())
        }
        fn SetUri(&self, _: &PCWSTR) -> Result<()> {
            Err(E_NOTIMPL.into())
        }
        fn Method(&self, _: *mut PWSTR) -> Result<()> {
            Err(E_NOTIMPL.into())
        }
        fn SetMethod(&self, _: &PCWSTR) -> Result<()> {
            Err(E_NOTIMPL.into())
        }
        fn Content(&self) -> Result<IStream> {
            // The generated COM implementation converts Error::empty() into
            // S_OK and leaves the caller's null out-parameter untouched.
            self.0.clone()
        }
        fn SetContent(&self, _: Ref<IStream>) -> Result<()> {
            Err(E_NOTIMPL.into())
        }
        fn Headers(&self) -> Result<ICoreWebView2HttpRequestHeaders> {
            Err(E_NOTIMPL.into())
        }
    }

    #[test]
    fn successful_null_body_is_not_a_transport_failure() {
        let request: ICoreWebView2WebResourceRequest = Request(Err(Error::empty())).into();
        // Reproduce the generated getter's surprising success-code error.
        // SAFETY: Locally implemented live COM request, no native window.
        assert_eq!(unsafe { request.Content() }.unwrap_err().code(), S_OK);
        assert!(content(&request).unwrap().is_none());
    }

    #[test]
    fn real_content_failures_are_not_silently_empty() {
        for code in [E_FAIL, E_POINTER] {
            let request: ICoreWebView2WebResourceRequest = Request(Err(code.into())).into();
            assert_eq!(content(&request).unwrap_err().code(), code);
        }
    }

    #[test]
    fn present_body_keeps_its_owned_stream_reference() {
        let stream: IStream = super::super::protocol::MemoryStream::new(vec![1, 2, 3]).into();
        let request: ICoreWebView2WebResourceRequest = Request(Ok(stream.clone())).into();
        let body = content(&request).unwrap().unwrap();
        drop(request);
        assert_eq!(body, stream);
    }
}
