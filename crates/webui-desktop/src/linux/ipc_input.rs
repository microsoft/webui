// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::ipc::{IpcBridge, IpcError, IpcErrorCode, IpcInputPermit, OwnedIpcHttpRequest};

#[derive(Clone, Copy)]
pub(super) struct ReadLimits {
    pub(super) maximum: usize,
    pub(super) control: usize,
    pub(super) declared: Option<usize>,
}

// GIO owns this buffer until its read callback runs, including cancellation.
// The reservation must move with the allocation into read_future.
pub(super) struct ReadBuffer {
    pub(super) bytes: Vec<u8>,
    _permit: IpcInputPermit,
}

impl ReadBuffer {
    pub(super) fn new(bridge: &IpcBridge, maximum: usize) -> Result<Self, IpcError> {
        // Every valid configuration reserves at least 128 envelope bytes plus
        // error text. Never require payload-sized scratch to receive a control.
        Self::allocate(bridge, maximum.min(128))
    }

    fn allocate(bridge: &IpcBridge, chunk: usize) -> Result<Self, IpcError> {
        let mut permit = bridge.reserve_input(chunk)?;
        let mut bytes = Vec::with_capacity(chunk);
        if bytes.capacity() > chunk {
            permit.try_grow(bytes.capacity() - chunk)?;
        }
        bytes.resize(chunk, 0);
        Ok(Self {
            bytes,
            _permit: permit,
        })
    }

    pub(super) fn grow_for_payload(
        &mut self,
        bridge: &IpcBridge,
        received: usize,
        limits: ReadLimits,
    ) -> Result<(), IpcError> {
        let target = limits.maximum.min(16 * 1024);
        if received <= limits.control || self.bytes.len() >= target {
            return Ok(());
        }
        // Acquire a new ordinary reservation rather than growing an emergency
        // permit beyond its control ceiling. Both allocations stay charged
        // until replacement drops the old storage.
        let replacement = Self::allocate(bridge, target)?;
        *self = replacement;
        Ok(())
    }
}

pub(super) fn append(
    input: &mut OwnedIpcHttpRequest,
    bytes: &[u8],
    limits: ReadLimits,
) -> Result<(), IpcError> {
    let length = input
        .body
        .len()
        .checked_add(bytes.len())
        .ok_or_else(|| error(IpcErrorCode::PayloadTooLarge))?;
    if length > limits.maximum {
        return Err(error(IpcErrorCode::PayloadTooLarge));
    }
    if limits.declared.is_some_and(|declared| length > declared) {
        return Err(error(IpcErrorCode::InvalidFrame));
    }
    let capacity = input.body.capacity();
    if length > capacity {
        let ceiling = if length <= limits.control {
            limits.control
        } else {
            limits.maximum
        };
        let target = length.max(capacity.saturating_mul(2)).min(ceiling);
        input.input_permit.try_grow(target - capacity)?;
        input.body.reserve_exact(target - input.body.len());
        if input.body.capacity() > target {
            input
                .input_permit
                .try_grow(input.body.capacity() - target)?;
        }
    }
    input.body.extend_from_slice(bytes);
    Ok(())
}

#[cold]
fn error(code: IpcErrorCode) -> IpcError {
    IpcError::new(
        code,
        "native IPC body exceeds its admitted bounds",
        "send a complete frame within the configured size and declared length",
    )
}

impl AsMut<[u8]> for ReadBuffer {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::ipc::{IpcHost, IpcLimits, IpcOptions, IpcRegistry, IpcWindowOwner};
    use crate::DesktopHttpMethod;
    use std::sync::Arc;

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
    fn tiny_control_scratch_remains_available_under_payload_saturation() {
        let limits = IpcLimits::default();
        let owner = IpcWindowOwner::new(
            Arc::new(IpcRegistry::default()),
            IpcOptions::default(),
            IpcHost::Source {
                origin: "webui://app".into(),
            },
        )
        .unwrap();
        let bridge = owner.bridge();
        let held: Vec<_> = (0..limits.max_admitted_input_bytes_per_frame / limits.max_frame_bytes)
            .map(|_| bridge.reserve_input(limits.max_frame_bytes).unwrap())
            .collect();
        assert!(bridge.reserve_input(24).is_ok());
        let scratch = ReadBuffer::new(&bridge, limits.max_frame_bytes);
        assert!(
            scratch.is_ok(),
            "control read rejected: {:?}",
            scratch.as_ref().err().map(|error| error.code)
        );
        drop((held, scratch));
    }

    #[test]
    fn maximum_control_body_and_scratch_fit_emergency_reserve_without_payload_growth() {
        for error_text_bytes in [1, 500, 2048] {
            let options = IpcOptions {
                limits: IpcLimits {
                    max_error_text_bytes_total: error_text_bytes,
                    ..IpcLimits::default()
                },
                ..IpcOptions::default()
            };
            let limits = ReadLimits {
                maximum: options.limits.max_frame_bytes,
                control: error_text_bytes + 128,
                // A dishonest large header must not demand payload scratch.
                declared: Some(options.limits.max_frame_bytes),
            };
            let owner = IpcWindowOwner::new(
                Arc::new(IpcRegistry::default()),
                options,
                IpcHost::Source {
                    origin: "webui://app".into(),
                },
            )
            .unwrap();
            let bridge = owner.bridge();
            let held: Vec<_> = (0..8)
                .map(|_| bridge.reserve_input(limits.maximum).unwrap())
                .collect();
            let mut body = input(&bridge);
            let mut scratch = ReadBuffer::new(&bridge, limits.maximum).unwrap();
            while body.body.len() < limits.control {
                let count = scratch.bytes.len().min(limits.control - body.body.len());
                append(&mut body, &scratch.bytes[..count], limits).unwrap();
                scratch
                    .grow_for_payload(&bridge, body.body.len(), limits)
                    .unwrap();
                assert_eq!(scratch.bytes.len(), 128);
            }
            assert_eq!(body.body.len(), limits.control);
            assert!(body.body.capacity() <= limits.control);
            assert!(append(&mut body, &[0], limits).is_err());
            assert!(scratch
                .grow_for_payload(&bridge, limits.control + 1, limits)
                .is_err());
            assert_eq!(scratch.bytes.len(), 128);
            drop(held);
            scratch
                .grow_for_payload(&bridge, limits.control + 1, limits)
                .unwrap();
            assert_eq!(scratch.bytes.len(), 16 * 1024);
        }
    }

    #[test]
    fn cancelled_io_buffer_keeps_its_credit_until_the_native_callback_releases_it() {
        let limits = IpcLimits::default();
        let owner = IpcWindowOwner::new(
            Arc::new(IpcRegistry::default()),
            IpcOptions::default(),
            IpcHost::Source {
                origin: "webui://app".into(),
            },
        )
        .unwrap();
        let bridge = owner.bridge();
        let held: Vec<_> = (0..8)
            .map(|_| bridge.reserve_input(limits.max_frame_bytes).unwrap())
            .collect();
        let scratch = ReadBuffer::new(&bridge, limits.max_frame_bytes).unwrap();
        let pointer = scratch.bytes.as_ptr();
        // Model read_future transferring ownership into GIO. Cancelling the
        // waiter cannot release this buffer before the completion callback.
        let native_callback_buffer = Some(scratch);
        let mut emergency = Vec::new();
        for _ in 0..10_000 {
            match bridge.reserve_input(128) {
                Ok(permit) => emergency.push(permit),
                Err(_) => break,
            }
        }
        assert!(bridge.reserve_input(128).is_err());
        assert_eq!(
            native_callback_buffer.as_ref().unwrap().bytes.as_ptr(),
            pointer
        );
        drop(native_callback_buffer);
        assert!(bridge.reserve_input(128).is_ok());
        drop((held, emergency));
    }
}
