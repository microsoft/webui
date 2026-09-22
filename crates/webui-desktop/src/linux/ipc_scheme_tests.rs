// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;
use crate::ipc::{IpcHost, IpcOptions, IpcRegistry, IpcWindowOwner};
use std::sync::Arc;

fn owner() -> IpcWindowOwner {
    IpcWindowOwner::new(
        Arc::new(IpcRegistry::default()),
        IpcOptions::default(),
        IpcHost::Source {
            origin: "webui://app".into(),
        },
    )
    .unwrap()
}

fn input(bridge: &IpcBridge) -> OwnedIpcHttpRequest {
    OwnedIpcHttpRequest {
        navigation: 1,
        method: DesktopHttpMethod::Post,
        path: "/_webui/ipc".into(),
        token: "a".repeat(32),
        body: Vec::new(),
        input_permit: bridge.reserve_input(0).unwrap(),
    }
}

#[test]
fn asynchronous_stream_bounds_actual_bytes_and_rejects_header_lies() {
    let owner = owner();
    let bridge = owner.bridge();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            for declared in [None, Some(3), Some(2), Some(4)] {
                let stream: gio::InputStream =
                    gio::MemoryInputStream::from_bytes(&glib::Bytes::from_static(&[1, 2, 3]))
                        .upcast();
                let result = context.block_on(read_body(
                    &bridge,
                    input(&bridge),
                    Some(stream),
                    ReadLimits {
                        maximum: 1024,
                        control: 512,
                        declared,
                    },
                ));
                if declared.is_none() || declared == Some(3) {
                    assert_eq!(result.unwrap().body, [1, 2, 3]);
                } else {
                    assert_eq!(result.err().unwrap().code, IpcErrorCode::InvalidFrame);
                }
            }
            let stream: gio::InputStream =
                gio::MemoryInputStream::from_bytes(&glib::Bytes::from_static(&[1, 2, 3])).upcast();
            assert_eq!(
                context
                    .block_on(read_body(
                        &bridge,
                        input(&bridge),
                        Some(stream),
                        ReadLimits {
                            maximum: 2,
                            control: 2,
                            declared: None
                        }
                    ))
                    .err()
                    .unwrap()
                    .code,
                IpcErrorCode::PayloadTooLarge
            );
        })
        .unwrap();
}

#[test]
fn native_headers_are_bounded_before_copy_and_sdk_assets_bypass_ipc() {
    let headers = webkit6::soup::MessageHeaders::new(webkit6::soup::MessageHeadersType::Request);
    headers.append("X-Test", &"a".repeat(33));
    assert_eq!(
        header(&headers, c"X-Test", 32).unwrap_err().code,
        IpcErrorCode::PayloadTooLarge
    );
    assert_eq!(header(&headers, c"Missing", 32).unwrap(), None);
    assert!(!is_ipc_path("/_webui/ipc/runtime.js"));
    assert!(!is_ipc_path("/_webui/ipc/bootstrap.js"));
    assert!(is_ipc_path("/_webui/ipc/outbound"));
}

#[test]
fn tiny_gio_control_read_succeeds_while_payload_bytes_are_saturated() {
    let owner = owner();
    let bridge = owner.bridge();
    let held: Vec<_> = (0..8)
        .map(|_| bridge.reserve_input(1_048_576).unwrap())
        .collect();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            for declared in [None, Some(24)] {
                let stream: gio::InputStream =
                    gio::MemoryInputStream::from_bytes(&glib::Bytes::from_static(&[0; 24]))
                        .upcast();
                let read = context
                    .block_on(read_body(
                        &bridge,
                        input(&bridge),
                        Some(stream),
                        ReadLimits {
                            maximum: 1_048_576,
                            control: 2176,
                            declared,
                        },
                    ))
                    .unwrap();
                assert_eq!(read.body, [0; 24]);
            }
        })
        .unwrap();
    drop(held);
}
