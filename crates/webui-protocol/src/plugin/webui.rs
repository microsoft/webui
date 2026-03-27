// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::{ProtocolError, Result};

const WEBUI_ELEMENT_DATA_LEN: usize = 12;

/// WebUI hydration element metadata encoded in plugin fragments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebUIElementData {
    /// Number of dynamic attribute bindings on the element.
    pub binding_count: u32,
    /// Starting event index for this element within the fragment-local event list.
    pub event_start: u32,
    /// Number of framework event handlers on the element.
    pub event_count: u32,
}

impl WebUIElementData {
    /// Encode this metadata using the WebUI 12-byte little-endian wire format.
    #[must_use]
    pub fn encode(self) -> [u8; WEBUI_ELEMENT_DATA_LEN] {
        let mut data = [0u8; WEBUI_ELEMENT_DATA_LEN];
        data[..4].copy_from_slice(&self.binding_count.to_le_bytes());
        data[4..8].copy_from_slice(&self.event_start.to_le_bytes());
        data[8..12].copy_from_slice(&self.event_count.to_le_bytes());
        data
    }

    /// Decode WebUI hydration metadata from protocol bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::Validation`] when the payload length is not 12 bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != WEBUI_ELEMENT_DATA_LEN {
            return Err(ProtocolError::Validation(format!(
                "WebUI element data must be {WEBUI_ELEMENT_DATA_LEN} bytes, received {}",
                bytes.len()
            )));
        }

        Ok(Self {
            binding_count: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            event_start: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            event_count: u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::WebUIElementData;
    use crate::ProtocolError;

    #[test]
    fn test_webui_element_data_roundtrip() {
        let encoded = WebUIElementData {
            binding_count: 2,
            event_start: 5,
            event_count: 1,
        }
        .encode();
        let decoded = WebUIElementData::decode(&encoded).expect("decode should succeed");
        assert_eq!(decoded.binding_count, 2);
        assert_eq!(decoded.event_start, 5);
        assert_eq!(decoded.event_count, 1);
    }

    #[test]
    fn test_webui_element_data_rejects_invalid_length() {
        let result = WebUIElementData::decode(&[1, 2, 3, 4]);
        assert!(
            matches!(result, Err(ProtocolError::Validation(ref msg)) if msg.contains("12 bytes")),
            "invalid payload length should be rejected: {result:?}"
        );
    }
}
