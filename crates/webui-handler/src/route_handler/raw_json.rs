// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use memchr::{memchr, memchr2, memchr2_iter};
use serde_json::value::RawValue;

pub(super) fn validate_literals(value: &RawValue) -> serde_json::Result<()> {
    let json = value.get();
    let bytes = json.as_bytes();
    validate_unicode(json)?;
    if !needs_number_check(bytes) {
        return Ok(());
    }
    let mut cursor = 0;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'"' => cursor = string_end(bytes, cursor)?,
            b'-' | b'0'..=b'9' => {
                let start = cursor;
                let mut exponent = false;
                while cursor < bytes.len()
                    && matches!(
                        bytes[cursor],
                        b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E'
                    )
                {
                    exponent |= matches!(bytes[cursor], b'e' | b'E');
                    cursor += 1;
                }
                // RawValue already proved the grammar. Without an exponent,
                // fewer than 309 bytes cannot exceed the finite f64 range.
                if exponent || cursor - start >= 309 {
                    serde_json::from_str::<f64>(&json[start..cursor])?;
                }
            }
            _ => cursor += 1,
        }
    }
    Ok(())
}

fn needs_number_check(bytes: &[u8]) -> bool {
    // Without a complete aligned 64-digit block, every digit run is at
    // most 126 digits. Even a two-digit exponent then remains safely finite.
    if bytes
        .as_chunks::<64>()
        .0
        .iter()
        .any(|chunk| chunk.iter().all(u8::is_ascii_digit))
    {
        return true;
    }
    // Reduce exponent candidates in one pass rather than restarting a
    // search at every 'e' in ordinary string contents. The non-short-circuiting
    // reduction lets the compiler vectorize this conservative preflight.
    // String contents may trigger the full check but cannot bypass it.
    let candidates = bytes.windows(2).fold(0u8, |found, pair| {
        found
            | u8::from(
                matches!(pair[0], b'e' | b'E') & matches!(pair[1], b'0'..=b'9' | b'+' | b'-'),
            )
    });
    candidates != 0 && has_large_exponent(bytes)
}

fn has_large_exponent(bytes: &[u8]) -> bool {
    // Unicode escapes such as \u1E00 are not large exponents. Retain the
    // precise check on this less common path, without scanning every ordinary
    // 'e' on the overwhelmingly common no-candidate path.
    for position in memchr2_iter(b'e', b'E', bytes) {
        if position == 0 || !bytes[position - 1].is_ascii_digit() {
            continue;
        }
        let mut start = position + 1;
        if bytes
            .get(start)
            .is_some_and(|byte| matches!(byte, b'-' | b'+'))
        {
            start += 1;
        }
        if bytes
            .get(start..start + 3)
            .is_some_and(|digits| digits.iter().all(u8::is_ascii_digit))
        {
            return true;
        }
    }
    false
}

fn validate_unicode(json: &str) -> serde_json::Result<()> {
    let bytes = json.as_bytes();
    let mut cursor = 0;
    while let Some(offset) = memchr(b'\\', &bytes[cursor..]) {
        cursor += offset;
        if bytes[cursor + 1] != b'u' {
            cursor += 2;
            continue;
        }
        let unit = unicode_unit(json, cursor + 2)?;
        cursor += 6;
        match unit {
            0xD800..=0xDBFF => {
                if bytes.get(cursor..cursor + 2) != Some(b"\\u")
                    || !matches!(unicode_unit(json, cursor + 2)?, 0xDC00..=0xDFFF)
                {
                    return Err(invalid_unicode());
                }
                cursor += 6;
            }
            0xDC00..=0xDFFF => return Err(invalid_unicode()),
            _ => {}
        }
    }
    Ok(())
}

fn unicode_unit(json: &str, start: usize) -> serde_json::Result<u16> {
    json.get(start..start + 4)
        .and_then(|hex| u16::from_str_radix(hex, 16).ok())
        .ok_or_else(invalid_unicode)
}

fn string_end(bytes: &[u8], start: usize) -> serde_json::Result<usize> {
    let mut cursor = start + 1;
    while let Some(offset) = memchr2(b'"', b'\\', &bytes[cursor..]) {
        cursor += offset + 1;
        if bytes[cursor - 1] == b'"' {
            return Ok(cursor);
        }
        cursor += 1;
    }
    Err(<serde_json::Error as serde::de::Error>::custom(
        "unterminated raw JSON string",
    ))
}

#[cold]
#[inline(never)]
fn invalid_unicode() -> serde_json::Error {
    serde::de::Error::custom("invalid Unicode surrogate pair")
}
