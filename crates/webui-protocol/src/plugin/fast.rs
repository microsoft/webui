// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::{ProtocolError, Result};

const FAST_ELEMENT_DATA_LEN: usize = 4;

/// FAST hydration element metadata encoded in plugin fragments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FastElementData {
    /// Number of dynamic attribute bindings on the element.
    pub binding_count: u32,
}

impl FastElementData {
    /// Encode this metadata using the FAST 4-byte little-endian wire format.
    #[must_use]
    pub fn encode(self) -> [u8; FAST_ELEMENT_DATA_LEN] {
        self.binding_count.to_le_bytes()
    }

    /// Decode FAST hydration metadata from protocol bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::Validation`] when the payload length is not 4 bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != FAST_ELEMENT_DATA_LEN {
            return Err(ProtocolError::Validation(format!(
                "FAST element data must be {FAST_ELEMENT_DATA_LEN} bytes, received {}",
                bytes.len()
            )));
        }

        Ok(Self {
            binding_count: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::FastElementData;
    use crate::ProtocolError;

    #[test]
    fn test_fast_element_data_roundtrip() {
        let encoded = FastElementData { binding_count: 3 }.encode();
        let decoded = FastElementData::decode(&encoded).expect("decode should succeed");
        assert_eq!(decoded.binding_count, 3);
    }

    #[test]
    fn test_fast_element_data_rejects_invalid_length() {
        let result = FastElementData::decode(&[1, 2]);
        assert!(
            matches!(result, Err(ProtocolError::Validation(ref msg)) if msg.contains("4 bytes")),
            "invalid payload length should be rejected: {result:?}"
        );
    }
}
