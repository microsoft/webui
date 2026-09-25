// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use std::{sync::Arc, time::Duration};
use webui_desktop::ipc::{
    decode_frame, IpcErrorCode, IpcHost, IpcLimits, IpcOptions, IpcRegistry, IpcWindowOwner,
    DEFAULT_MAX_IPC_PAYLOAD_BYTES,
};

#[test]
fn default_policy_and_checked_builders_are_usable_by_external_hosts() {
    let defaults = IpcLimits::default();
    assert_eq!(defaults.max_frame_bytes(), DEFAULT_MAX_IPC_PAYLOAD_BYTES);
    assert_eq!(defaults.default_timeout(), Duration::from_secs(30));
    let policy = defaults
        .with_max_frame_bytes(256 * 1024)
        .unwrap()
        .with_default_timeout(Duration::from_secs(10))
        .unwrap();
    assert_eq!(policy.max_frame_bytes(), 256 * 1024);
    assert_eq!(policy.default_timeout(), Duration::from_secs(10));
    let owner = IpcWindowOwner::new(
        Arc::new(IpcRegistry::default()),
        IpcOptions {
            limits: policy.clone(),
            ..IpcOptions::default()
        },
        IpcHost::Packaged {
            origin: "webui://app".into(),
        },
    )
    .unwrap();
    assert!(!owner.window().stats().workers_started);
    assert_eq!(
        decode_frame(&vec![0; policy.max_frame_bytes() + 1], &policy)
            .unwrap_err()
            .code,
        IpcErrorCode::PayloadTooLarge
    );
}

#[test]
fn frame_policy_accepts_boundaries_and_rejects_unbounded_or_undersized_values() {
    for bytes in [2176, 8 * 1024 * 1024] {
        let policy = IpcLimits::default().with_max_frame_bytes(bytes).unwrap();
        assert_eq!(policy.max_frame_bytes(), bytes);
    }
    for bytes in [0, 2175, 8 * 1024 * 1024 + 1, usize::MAX] {
        let error = IpcLimits::default()
            .with_max_frame_bytes(bytes)
            .unwrap_err();
        assert_eq!(error.code, IpcErrorCode::InvalidPayload);
        assert!(error.help.contains("2176"));
    }
}

#[test]
fn timeout_policy_checks_precision_and_both_bounds() {
    for timeout in [Duration::from_millis(1), Duration::from_secs(300)] {
        let policy = IpcLimits::default().with_default_timeout(timeout).unwrap();
        assert_eq!(policy.default_timeout(), timeout);
    }
    for timeout in [
        Duration::ZERO,
        Duration::from_nanos(1),
        Duration::from_micros(1500),
        Duration::from_millis(300_001),
        Duration::MAX,
    ] {
        let error = IpcLimits::default()
            .with_default_timeout(timeout)
            .unwrap_err();
        assert_eq!(error.code, IpcErrorCode::InvalidPayload);
        assert!(error.help.contains("milliseconds"));
    }
}
