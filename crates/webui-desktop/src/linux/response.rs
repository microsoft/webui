// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::DesktopProtocolResponse;
use gtk4::{gio, glib};
use webkit6::{URISchemeRequest, URISchemeResponse};

pub(super) fn finish_scheme_request(request: &URISchemeRequest, response: DesktopProtocolResponse) {
    finish(request, response, false);
}

pub(super) fn finish_ipc_request(request: &URISchemeRequest, response: DesktopProtocolResponse) {
    finish(request, response, true);
}

fn finish(request: &URISchemeRequest, response: DesktopProtocolResponse, ipc: bool) {
    // GBytes owns the complete response, including any frame-ledger lease.
    // GIO/WebKit may retain this storage beyond finish_with_response.
    let bytes = glib::Bytes::from_owned(response.body);
    let stream = gio::MemoryInputStream::from_bytes(&bytes);
    let length = i64::try_from(bytes.len()).unwrap_or(-1);
    let scheme_response = URISchemeResponse::new(&stream, length);
    if ipc {
        let headers =
            webkit6::soup::MessageHeaders::new(webkit6::soup::MessageHeadersType::Response);
        headers.append("Cache-Control", "no-store");
        scheme_response.set_http_headers(headers);
    }
    scheme_response.set_content_type(&response.content_type);
    scheme_response.set_status(u32::from(response.status), None);
    request.finish_with_response(&scheme_response);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    struct ReleaseCount(Arc<AtomicUsize>);
    impl Drop for ReleaseCount {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn gio_stream_retains_whole_response_body_until_native_release() {
        let released = Arc::new(AtomicUsize::new(0));
        let body = vec![1, 2, 3];
        let pointer = body.as_ptr();
        let bytes = glib::Bytes::from_owned(crate::DesktopResponseBody::with_guard(
            body,
            ReleaseCount(Arc::clone(&released)),
        ));
        assert_eq!(bytes.as_ref().as_ptr(), pointer);
        let stream = gio::MemoryInputStream::from_bytes(&bytes);
        drop(bytes);
        assert_eq!(released.load(Ordering::SeqCst), 0);
        drop(stream);
        assert_eq!(released.load(Ordering::SeqCst), 1);
    }
}
