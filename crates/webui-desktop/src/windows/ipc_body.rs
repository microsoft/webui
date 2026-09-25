// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Stream-body policy shared by the Windows adapter and its portable regressions.

use crate::ipc::{IpcError, IpcErrorCode};

pub(super) fn check_claim(
    max_bytes: usize,
    claimed: Option<usize>,
    actual: usize,
) -> Result<(), IpcError> {
    if actual > max_bytes || claimed.is_some_and(|size| size > max_bytes) {
        return Err(error(IpcErrorCode::PayloadTooLarge));
    }
    if claimed.is_some_and(|size| size != actual) {
        return Err(error(IpcErrorCode::InvalidFrame));
    }
    Ok(())
}

/// Reserve the actual bytes read before growing or copying. Content-Length
/// never determines allocation size, and a one-byte overflow is rejected.
pub(super) fn read_body(
    max_bytes: usize,
    claimed: Option<usize>,
    mut read: impl FnMut(&mut [u8]) -> Result<usize, IpcError>,
    mut reserve: impl FnMut(usize) -> Result<(), IpcError>,
) -> Result<Vec<u8>, IpcError> {
    if claimed.is_some_and(|size| size > max_bytes) {
        return Err(error(IpcErrorCode::PayloadTooLarge));
    }
    let mut body = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let remaining = max_bytes.saturating_sub(body.len());
        let limit = buffer.len().min(remaining.saturating_add(1));
        let count = read(&mut buffer[..limit])?;
        if count > limit || count > remaining {
            return Err(error(IpcErrorCode::PayloadTooLarge));
        }
        if count == 0 {
            check_claim(max_bytes, claimed, body.len())?;
            return Ok(body);
        }
        let needed = body.len() + count;
        if needed > body.capacity() {
            // Geometric, fully charged growth avoids quadratic copies. The
            // first small control frame reserves only its actual byte count.
            let capacity = needed.max(body.capacity().saturating_mul(2)).min(max_bytes);
            reserve(capacity - body.capacity())?;
            body.try_reserve_exact(capacity - body.len())
                .map_err(|_| error(IpcErrorCode::Overloaded))?;
            // Reject an allocator that reports unreserved excess capacity.
            if body.capacity() > capacity {
                return Err(error(IpcErrorCode::Overloaded));
            }
        }
        body.extend_from_slice(&buffer[..count]);
    }
}

#[cold]
#[inline(never)]
fn error(code: IpcErrorCode) -> IpcError {
    IpcError::new(
        code,
        code.as_str(),
        "send a frame within the configured limit or reduce concurrent IPC work",
    )
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::ipc::{IpcHost, IpcLimits, IpcOptions, IpcRegistry, IpcWindowOwner};
    use std::cell::Cell;
    use std::sync::Arc;

    fn configured_owner(max_bytes: usize) -> IpcWindowOwner {
        let options = IpcOptions {
            limits: IpcLimits {
                max_frame_bytes: max_bytes,
                ..IpcLimits::default()
            },
            ..IpcOptions::default()
        };
        IpcWindowOwner::new(
            Arc::new(IpcRegistry::default()),
            options,
            IpcHost::Packaged {
                origin: "https://app.webui.localhost".into(),
            },
        )
        .unwrap()
    }

    #[test]
    fn validated_two_mib_limit_accepts_frame_above_default() {
        let max_bytes = 2 * 1024 * 1024;
        let owner = configured_owner(max_bytes);
        let bridge = owner.bridge();
        let mut permit = None;
        let size = 1_048_577;
        let mut remaining = size;
        let body = read_body(
            max_bytes,
            Some(size),
            |buffer| {
                let count = remaining.min(buffer.len());
                remaining -= count;
                buffer[..count].fill(7);
                Ok(count)
            },
            |bytes| {
                if let Some(permit) = &mut permit {
                    crate::ipc::IpcInputPermit::try_grow(permit, bytes)
                } else {
                    permit = Some(bridge.reserve_input(bytes)?);
                    Ok(())
                }
            },
        )
        .unwrap();
        assert_eq!(body.len(), size);
        assert!(body.iter().all(|byte| *byte == 7));
        assert!(check_claim(max_bytes, Some(size), body.len()).is_ok());
        let capacity = body.capacity();
        assert_eq!(owner.window().stats().admitted_input_bytes, capacity);
        assert_eq!(owner.window().stats().retained_bytes, capacity);
        drop(body);
        assert_eq!(owner.window().stats().admitted_input_bytes, capacity);
        drop(permit);
        assert_eq!(owner.window().stats().admitted_input_bytes, 0);
        assert_eq!(owner.window().stats().retained_bytes, 0);
    }

    #[test]
    fn default_lower_and_larger_limits_are_inclusive_and_fully_charged() {
        assert_eq!(IpcLimits::default().max_frame_bytes, 1_048_576);
        for max_bytes in [
            4096,
            8193,
            IpcLimits::default().max_frame_bytes,
            2 * 1024 * 1024,
        ] {
            let _owner = configured_owner(max_bytes);
            for size in [0, max_bytes - 1, max_bytes, max_bytes + 1] {
                for claimed in [None, Some(size)] {
                    let mut remaining = size;
                    let mut charged = 0;
                    let reads = Cell::new(0);
                    let result = read_body(
                        max_bytes,
                        claimed,
                        |buffer| {
                            reads.set(reads.get() + 1);
                            let count = remaining.min(buffer.len());
                            remaining -= count;
                            buffer[..count].fill(9);
                            Ok(count)
                        },
                        |bytes| {
                            charged += bytes;
                            Ok(())
                        },
                    );
                    if size <= max_bytes {
                        let body = result.unwrap();
                        assert_eq!(body.len(), size);
                        assert_eq!(body.capacity(), charged);
                        assert!(body.iter().all(|byte| *byte == 9));
                    } else {
                        assert_eq!(result.unwrap_err().code, IpcErrorCode::PayloadTooLarge);
                        if claimed.is_some() {
                            assert_eq!(reads.get(), 0);
                            assert_eq!(charged, 0);
                        }
                    }
                    assert!(charged <= max_bytes);
                }
            }
        }
    }

    #[test]
    fn false_headers_and_credit_failure_remain_explicit() {
        for max_bytes in [4096, 2 * 1024 * 1024] {
            for claimed in [Some(0), Some(2)] {
                let mut remaining = 1;
                let result = read_body(
                    max_bytes,
                    claimed,
                    |buffer| {
                        let count = remaining;
                        remaining = 0;
                        buffer[..count].fill(1);
                        Ok(count)
                    },
                    |_| Ok(()),
                );
                assert_eq!(result.unwrap_err().code, IpcErrorCode::InvalidFrame);
            }
            let mut reads = 0;
            let result = read_body(
                max_bytes,
                None,
                |buffer| {
                    reads += 1;
                    buffer[0] = 1;
                    Ok(1)
                },
                |_| Err(error(IpcErrorCode::Overloaded)),
            );
            assert_eq!(result.unwrap_err().code, IpcErrorCode::Overloaded);
            assert_eq!(reads, 1);
        }
    }
}
