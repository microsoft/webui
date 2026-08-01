// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::WebUIError;

pub(super) fn push_json_string(
    out: &mut String,
    value: &str,
    context: &str,
) -> Result<(), WebUIError> {
    let _ = context;
    out.push('"');
    let bytes = value.as_bytes();
    let mut start = 0usize;
    for (index, byte) in bytes.iter().copied().enumerate() {
        let escape = match byte {
            b'"' => Some("\\\""),
            b'\\' => Some("\\\\"),
            b'\n' => Some("\\n"),
            b'\r' => Some("\\r"),
            b'\t' => Some("\\t"),
            0x08 => Some("\\b"),
            0x0c => Some("\\f"),
            0x00..=0x1f => {
                if start < index {
                    out.push_str(&value[start..index]);
                }
                out.push_str("\\u00");
                const HEX: &[u8; 16] = b"0123456789abcdef";
                out.push(char::from(HEX[usize::from(byte >> 4)]));
                out.push(char::from(HEX[usize::from(byte & 0x0f)]));
                start = index + 1;
                None
            }
            _ => None,
        };
        if let Some(escape) = escape {
            if start < index {
                out.push_str(&value[start..index]);
            }
            out.push_str(escape);
            start = index + 1;
        }
    }
    if start < value.len() {
        out.push_str(&value[start..]);
    }
    out.push('"');
    Ok(())
}

pub(super) fn encode_json_string(value: &str, context: &str) -> Result<String, WebUIError> {
    serde_json::to_string(value)
        .map_err(|error| WebUIError::Serialization(format!("Failed to encode {context}: {error}")))
}

pub(super) fn push_u64(out: &mut String, value: u64) {
    let mut digits = [0u8; 20];
    let mut n = value;
    let mut i = digits.len();
    if n == 0 {
        out.push('0');
        return;
    }
    while n > 0 {
        i -= 1;
        digits[i] = match n % 10 {
            0 => b'0',
            1 => b'1',
            2 => b'2',
            3 => b'3',
            4 => b'4',
            5 => b'5',
            6 => b'6',
            7 => b'7',
            8 => b'8',
            _ => b'9',
        };
        n /= 10;
    }
    for digit in &digits[i..] {
        out.push(char::from(*digit));
    }
}

#[cfg(test)]
mod tests {
    use super::push_json_string;

    #[test]
    fn custom_json_string_encoding_matches_serde() -> Result<(), Box<dyn std::error::Error>> {
        let controls: String = (0u8..=0x1f).map(char::from).collect();
        let cases = [
            "",
            "plain ASCII",
            "\"quoted\" and \\\\ slashed /",
            "\u{0008}\u{000c}\n\r\t",
            controls.as_str(),
            "caf\u{00e9} \u{4e2d}\u{6587} \u{1f600} \u{2028}\u{2029}",
            "</script>",
        ];

        for value in cases {
            let mut actual = String::new();
            push_json_string(&mut actual, value, "test value")?;
            assert_eq!(actual, serde_json::to_string(value)?);
        }
        Ok(())
    }
}
