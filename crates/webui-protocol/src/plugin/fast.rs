// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::{ProtocolError, Result};

const FAST_ELEMENT_DATA_LEN: usize = 4;
const FAST_V2_ELEMENT_DATA_LEN: usize = 5;
const FAST_V2_RESET_CHILD_SCOPE: u8 = 1;

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

    /// Encode FAST 2 metadata with an optional child-scope lifecycle flag.
    #[must_use]
    pub fn encode_v2(self, reset_child_scope: bool) -> [u8; FAST_V2_ELEMENT_DATA_LEN] {
        let mut data = [0; FAST_V2_ELEMENT_DATA_LEN];
        data[..FAST_ELEMENT_DATA_LEN].copy_from_slice(&self.binding_count.to_le_bytes());
        if reset_child_scope {
            data[FAST_ELEMENT_DATA_LEN] = FAST_V2_RESET_CHILD_SCOPE;
        }
        data
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

    /// Decode FAST 2 element metadata.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::Validation`] for unsupported payload lengths or flags.
    pub fn decode_v2(bytes: &[u8]) -> Result<(Self, bool)> {
        let [a, b, c, d, flags] = bytes else {
            return Err(ProtocolError::Validation(format!(
                "FAST 2 element data must be {FAST_V2_ELEMENT_DATA_LEN} bytes, received {}",
                bytes.len()
            )));
        };
        let binding_count = u32::from_le_bytes([*a, *b, *c, *d]);
        if *flags & !FAST_V2_RESET_CHILD_SCOPE != 0 {
            return Err(ProtocolError::Validation(format!(
                "FAST 2 element data contains unsupported flags: 0x{flags:02x}"
            )));
        }
        Ok((
            Self { binding_count },
            *flags & FAST_V2_RESET_CHILD_SCOPE != 0,
        ))
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

    #[test]
    fn test_fast_v2_element_data_roundtrip() {
        let encoded = FastElementData { binding_count: 3 }.encode_v2(true);
        let decoded = FastElementData::decode_v2(&encoded).expect("decode should succeed");
        assert_eq!(decoded, (FastElementData { binding_count: 3 }, true));
    }

    #[test]
    fn test_fast_v2_element_data_rejects_four_byte_count() {
        let result = FastElementData::decode_v2(&3u32.to_le_bytes());
        assert!(
            matches!(result, Err(ProtocolError::Validation(ref msg)) if msg.contains("5 bytes")),
            "four-byte FAST 2 data should be rejected: {result:?}"
        );
    }

    #[test]
    fn test_fast_v2_element_data_rejects_unknown_flags() {
        let result = FastElementData::decode_v2(&[3, 0, 0, 0, 2]);
        assert!(
            matches!(result, Err(ProtocolError::Validation(ref msg)) if msg.contains("unsupported flags")),
            "unknown flags should be rejected: {result:?}"
        );
    }
}
