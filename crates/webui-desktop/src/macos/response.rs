// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::ffi::c_void;
use std::ptr::NonNull;

use crate::{DesktopProtocolResponse, DesktopResponseBody};
use block2::RcBlock;
use objc2::ffi::NSInteger;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::ProtocolObject;
use objc2::{extern_class, extern_conformance, extern_methods, AnyThread};
use objc2_foundation::{
    ns_string, NSData, NSDictionary, NSObjectProtocol, NSString, NSURLResponse, NSURL,
};
use objc2_web_kit::WKURLSchemeTask;

extern_class!(
    // HTTP response subclass used to preserve status codes for custom-scheme loads.
    #[unsafe(super(NSURLResponse))]
    #[derive(Debug, PartialEq, Eq, Hash)]
    struct NSHTTPURLResponse;
);

extern_conformance!(
    // SAFETY: NSHTTPURLResponse inherits NSObjectProtocol conformance from Foundation.
    unsafe impl NSObjectProtocol for NSHTTPURLResponse {}
);

#[allow(non_snake_case)]
impl NSHTTPURLResponse {
    extern_methods!(
        #[unsafe(method(initWithURL:statusCode:HTTPVersion:headerFields:))]
        #[unsafe(method_family = init)]
        fn initWithURL_statusCode_HTTPVersion_headerFields(
            this: Allocated<Self>,
            url: &NSURL,
            status_code: NSInteger,
            http_version: Option<&NSString>,
            header_fields: Option<&NSDictionary<NSString, NSString>>,
        ) -> Retained<Self>;
    );
}

pub(super) fn send_response(
    task: &ProtocolObject<dyn WKURLSchemeTask>,
    url: Option<&NSURL>,
    response: DesktopProtocolResponse,
) {
    send_response_cancellable(task, url, response, || true);
}

pub(super) fn send_response_cancellable(
    task: &ProtocolObject<dyn WKURLSchemeTask>,
    url: Option<&NSURL>,
    response: DesktopProtocolResponse,
    is_live: impl Fn() -> bool,
) {
    let fallback_url = url
        .is_none()
        .then(|| NSURL::URLWithString(ns_string!("webui://app/")))
        .flatten();
    let Some(url) = url.or(fallback_url.as_deref()) else {
        return;
    };
    let content_type = NSString::from_str(&response.content_type);
    let headers = NSDictionary::from_slices(
        &[
            ns_string!("Content-Type"),
            ns_string!("Cache-Control"),
            ns_string!("Pragma"),
            ns_string!("Expires"),
        ],
        &[
            &*content_type,
            ns_string!("no-store, no-cache, must-revalidate"),
            ns_string!("no-cache"),
            ns_string!("0"),
        ],
    );
    let http_response = NSHTTPURLResponse::initWithURL_statusCode_HTTPVersion_headerFields(
        NSHTTPURLResponse::alloc(),
        url,
        NSInteger::try_from(u32::from(response.status)).unwrap_or(500),
        Some(ns_string!("HTTP/1.1")),
        Some(&headers),
    );
    // SAFETY: The task is live for the callback. WebKit requires the response,
    // optional data, then completion, and may retain the data beyond this call.
    unsafe {
        if !is_live() {
            return;
        }
        task.didReceiveResponse(&http_response);
        if !is_live() {
            return;
        }
        if !response.body.is_empty() {
            let data = native_data(response.body);
            task.didReceiveData(&data);
        }
        if is_live() {
            task.didFinish();
        }
    }
}

fn native_data(body: DesktopResponseBody) -> Retained<NSData> {
    let length = body.len();
    let bytes = NonNull::from(body.as_slice()).cast::<c_void>();
    // Capture the complete body, not just its Vec. Foundation retains this
    // block with its no-copy storage, so an IPC reservation cannot be returned
    // while WebKit still owns the bytes after didFinish or task cancellation.
    let deallocator = RcBlock::new(move |_bytes: NonNull<c_void>, _length: usize| {
        let _keep_body_until_block_release = &body;
    });
    // SAFETY: `bytes` points into the uniquely owned body captured by the block,
    // and is valid for `length` bytes. Immutable NSData never mutates it.
    // Foundation copies/retains the block before this call returns and releases
    // it only after relinquishing the no-copy storage. Dropping that capture
    // frees the original Vec with its original capacity and then its lease;
    // the callback must not additionally free or reconstruct the allocation.
    // DesktopResponseBody is Send + Sync, so final release on a WebKit thread
    // does not transfer UI-thread-local objects across threads.
    unsafe {
        NSData::initWithBytesNoCopy_length_deallocator(
            NSData::alloc(),
            bytes,
            length,
            Some(&deallocator),
        )
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use objc2::rc::autoreleasepool;
    use objc2::{define_class, msg_send, DefinedClass, Message};
    use objc2_foundation::{NSError, NSObject, NSURLRequest};

    #[derive(Default)]
    struct TaskIvars {
        response: RefCell<Option<Retained<NSURLResponse>>>,
        data: RefCell<Option<Retained<NSData>>>,
        calls: RefCell<Vec<&'static str>>,
    }

    define_class!(
        // SAFETY: This test double is an NSObject subclass with ordinary Rust ivars.
        #[unsafe(super = NSObject)]
        #[ivars = TaskIvars]
        struct TestSchemeTask;

        // SAFETY: NSObjectProtocol has no additional requirements.
        unsafe impl NSObjectProtocol for TestSchemeTask {}

        // SAFETY: Signatures match WKURLSchemeTask; callbacks retain their inputs.
        unsafe impl WKURLSchemeTask for TestSchemeTask {
            #[unsafe(method_id(request))]
            fn request(&self) -> Retained<NSURLRequest> {
                let url = NSURL::URLWithString(ns_string!("webui://app/asset")).unwrap();
                NSURLRequest::requestWithURL(&url)
            }

            #[unsafe(method(didReceiveResponse:))]
            fn did_receive_response(&self, response: &NSURLResponse) {
                *self.ivars().response.borrow_mut() = Some(response.retain());
                self.ivars().calls.borrow_mut().push("response");
            }

            #[unsafe(method(didReceiveData:))]
            fn did_receive_data(&self, data: &NSData) {
                *self.ivars().data.borrow_mut() = Some(data.retain());
                self.ivars().calls.borrow_mut().push("data");
            }

            #[unsafe(method(didFinish))]
            fn did_finish(&self) {
                self.ivars().calls.borrow_mut().push("finish");
            }

            #[unsafe(method(didFailWithError:))]
            fn did_fail(&self, _error: &NSError) {
                self.ivars().calls.borrow_mut().push("error");
            }
        }
    );

    impl TestSchemeTask {
        fn new() -> Retained<Self> {
            let this = Self::alloc().set_ivars(TaskIvars::default());
            // SAFETY: NSObject init has the expected signature for this test subclass.
            unsafe { msg_send![super(this), init] }
        }
    }

    struct ReleaseCount(Arc<AtomicUsize>);

    impl Drop for ReleaseCount {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn native_buffer_retains_response_lease_after_http_completion() {
        for cancel_after_data in [false, true] {
            autoreleasepool(|_| {
                let released = Arc::new(AtomicUsize::new(0));
                let task = TestSchemeTask::new();
                let mut bytes = Vec::with_capacity(8192);
                bytes.extend_from_slice(&[1, 2, 3]);
                let original_pointer = bytes.as_ptr();
                let body =
                    DesktopResponseBody::with_guard(bytes, ReleaseCount(Arc::clone(&released)));
                autoreleasepool(|_| {
                    send_response_cancellable(
                        ProtocolObject::from_ref(&*task),
                        None,
                        DesktopProtocolResponse::protobuf(body),
                        || !cancel_after_data || task.ivars().data.borrow().is_none(),
                    );
                });
                let expected: &[&str] = if cancel_after_data {
                    &["response", "data"]
                } else {
                    &["response", "data", "finish"]
                };
                assert_eq!(*task.ivars().calls.borrow(), expected);
                assert_eq!(released.load(Ordering::SeqCst), 0);
                let retained = task.ivars().data.borrow_mut().take().unwrap();
                // SAFETY: The native buffer is immutable and retained in this scope.
                assert_eq!(
                    unsafe { retained.as_bytes_unchecked() }.as_ptr(),
                    original_pointer
                );
                drop(task);
                assert_eq!(released.load(Ordering::SeqCst), 0);
                drop(retained);
                assert_eq!(released.load(Ordering::SeqCst), 1);
            });
        }
    }

    #[test]
    fn cancelled_and_empty_responses_release_without_native_buffer_ownership() {
        for (bytes, cancel) in [(vec![1, 2, 3], true), (Vec::new(), false)] {
            autoreleasepool(|_| {
                let released = Arc::new(AtomicUsize::new(0));
                let task = TestSchemeTask::new();
                send_response_cancellable(
                    ProtocolObject::from_ref(&*task),
                    None,
                    DesktopProtocolResponse::protobuf(DesktopResponseBody::with_guard(
                        bytes,
                        ReleaseCount(Arc::clone(&released)),
                    )),
                    || !cancel,
                );
                assert!(task.ivars().data.borrow().is_none());
                assert_eq!(released.load(Ordering::SeqCst), 1);
            });
        }
    }

    #[test]
    fn stopped_response_never_delivers_later_data_or_finish() {
        autoreleasepool(|_| {
            let task = TestSchemeTask::new();
            send_response_cancellable(
                ProtocolObject::from_ref(&*task),
                None,
                DesktopProtocolResponse::new(200, "application/x-protobuf", vec![1, 2, 3]),
                || task.ivars().calls.borrow().is_empty(),
            );
            assert_eq!(*task.ivars().calls.borrow(), ["response"]);
        });
    }

    #[test]
    fn response_transfers_buffer_and_preserves_retained_data() {
        autoreleasepool(|_| {
            let task = TestSchemeTask::new();
            let mut body = Vec::with_capacity(8192);
            body.extend((0..4096).map(|i| u8::try_from(i % 256).unwrap()));
            let original_pointer = body.as_ptr();
            autoreleasepool(|_| {
                send_response(
                    ProtocolObject::from_ref(&*task),
                    None,
                    DesktopProtocolResponse::new(201, "application/octet-stream", body),
                );
            });

            let retained = task.ivars().data.borrow();
            let data = retained.as_ref().unwrap();
            // SAFETY: This immutable NSData owns the moved Vec; nothing can mutate it.
            let bytes = unsafe { data.as_bytes_unchecked() };
            assert_eq!(bytes.as_ptr(), original_pointer);
            assert_eq!(bytes.len(), 4096);
            assert!(bytes
                .iter()
                .enumerate()
                .all(|(i, byte)| usize::from(*byte) == i % 256));
            assert_eq!(*task.ivars().calls.borrow(), ["response", "data", "finish"]);
            let response = task.ivars().response.borrow();
            let response = response.as_ref().unwrap();
            // SAFETY: send_response constructs an NSHTTPURLResponse with this selector.
            let status: NSInteger = unsafe { msg_send![&**response, statusCode] };
            assert_eq!(status, 201);
            assert_eq!(
                response.MIMEType().unwrap().to_string(),
                "application/octet-stream"
            );
            // SAFETY: The captured response is the NSHTTPURLResponse constructed above.
            let headers: Retained<NSDictionary<NSString, NSString>> =
                unsafe { msg_send![&**response, allHeaderFields] };
            assert_eq!(
                headers
                    .objectForKey(ns_string!("Cache-Control"))
                    .unwrap()
                    .to_string(),
                "no-store, no-cache, must-revalidate"
            );
            assert_eq!(
                headers
                    .objectForKey(ns_string!("Pragma"))
                    .unwrap()
                    .to_string(),
                "no-cache"
            );
            assert_eq!(
                headers
                    .objectForKey(ns_string!("Expires"))
                    .unwrap()
                    .to_string(),
                "0"
            );
        });
    }

    #[test]
    fn empty_response_finishes_without_data() {
        autoreleasepool(|_| {
            let task = TestSchemeTask::new();
            let url = NSURL::URLWithString(ns_string!("webui://app/empty")).unwrap();
            send_response(
                ProtocolObject::from_ref(&*task),
                Some(&url),
                DesktopProtocolResponse::text(204, ""),
            );
            assert_eq!(*task.ivars().calls.borrow(), ["response", "finish"]);
            assert!(task.ivars().data.borrow().is_none());
            assert_eq!(
                task.ivars()
                    .response
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .URL()
                    .unwrap()
                    .absoluteString()
                    .unwrap()
                    .to_string(),
                "webui://app/empty"
            );
        });
    }
}
