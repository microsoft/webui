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

// Number of decoded UTF-8 bytes in a validated, quoted string. Plain spans
// already are UTF-8; adjust the wire length only where an escape occurs.
pub(super) fn validated_string_len(quoted: &str) -> serde_json::Result<usize> {
    let raw = unquote(quoted)?;
    let mut len = raw.len();
    let mut rest = raw;
    while let Some(offset) = memchr(b'\\', rest.as_bytes()) {
        let escaped = &rest[offset + 1..];
        let (ch, consumed) = decoded_escape(escaped)?;
        len -= 1 + consumed - ch.len_utf8();
        rest = &escaped[consumed..];
    }
    Ok(len)
}

fn unquote(quoted: &str) -> serde_json::Result<&str> {
    quoted
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or_else(invalid_value)
}

// The leading backslash has already been consumed.
fn decoded_escape(escaped: &str) -> serde_json::Result<(char, usize)> {
    Ok(match escaped.as_bytes().first() {
        Some(b'u') => {
            let unit = unicode_unit(escaped, 1)?;
            if let Some(ch) = char::from_u32(u32::from(unit)) {
                (ch, 5)
            } else {
                let low = unicode_unit(escaped, 7)?;
                let ch = char::decode_utf16([unit, low])
                    .next()
                    .and_then(Result::ok)
                    .ok_or_else(invalid_unicode)?;
                (ch, 11)
            }
        }
        Some(b @ (b'"' | b'\\' | b'/')) => (char::from(*b), 1),
        Some(b'b') => ('\x08', 1),
        Some(b'f') => ('\x0c', 1),
        Some(b'n') => ('\n', 1),
        Some(b'r') => ('\r', 1),
        Some(b't') => ('\t', 1),
        _ => return Err(invalid_value()),
    })
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

// Delimit one value inside an already validated RawValue. The grammar has
// established matching delimiters, so even a discarded nested object only
// needs a depth counter, not a fresh serde deserializer or nesting stack.
pub(super) fn take_validated<'a>(input: &mut &'a str) -> serde_json::Result<&'a str> {
    let bytes = input.as_bytes();
    let end = match bytes.first() {
        Some(b'"') => string_end(bytes, 0)?,
        Some(b'{' | b'[') => {
            let mut depth = 1usize;
            let mut i = 1;
            while depth != 0 {
                match bytes.get(i) {
                    Some(b'"') => {
                        i = string_end(bytes, i)?;
                        continue;
                    }
                    Some(b'{' | b'[') => depth += 1,
                    Some(b'}' | b']') => depth -= 1,
                    Some(_) => {}
                    None => return Err(invalid_value()),
                }
                i += 1;
            }
            i
        }
        Some(b'-' | b'0'..=b'9' | b'n' | b't' | b'f') => bytes
            .iter()
            .position(|b| b.is_ascii_whitespace() || matches!(b, b',' | b']' | b'}'))
            .unwrap_or(bytes.len()),
        _ => return Err(invalid_value()),
    };
    let (value, rest) = input.split_at(end);
    *input = rest;
    Ok(value)
}

// Unescaped member names borrow the input. Escaped names use one buffer
// throughout the walk, bounded by the candidate compiled key's encoded size.
pub(super) fn validated_key<'a>(
    quoted: &'a str,
    scratch: &'a mut String,
) -> serde_json::Result<&'a str> {
    let raw = unquote(quoted)?;
    let Some((prefix, mut escaped)) = raw.split_once('\\') else {
        return Ok(raw);
    };
    scratch.clear();
    scratch.push_str(prefix);
    loop {
        let (ch, n) = decoded_escape(escaped)?;
        scratch.push(ch);
        let rest = &escaped[n..];
        if let Some((prefix, tail)) = rest.split_once('\\') {
            scratch.push_str(prefix);
            escaped = tail;
        } else {
            scratch.push_str(rest);
            return Ok(scratch);
        }
    }
}

#[cold]
fn invalid_value() -> serde_json::Error {
    serde::de::Error::custom("unexpected token in validated JSON value")
}

#[cold]
#[inline(never)]
fn invalid_unicode() -> serde_json::Error {
    serde::de::Error::custom("invalid Unicode surrogate pair")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validated_string_byte_count_matches_serde_for_plain_and_escaped_unicode(
    ) -> serde_json::Result<()> {
        for quoted in [
            r#""""#,
            r#""literal é界🙂""#,
            r#""\b\f\n\r\t\/\\\"""#,
            r#""\u0000\u007f\u0080\u07ff\u0800\uffff""#,
            r#""é\ud800\udc00界\udbff\udfff\/\u00e9𐀀""#,
            r#""\\uD83D\\uDE42\\\/""#,
        ] {
            assert_eq!(
                validated_string_len(quoted)?,
                serde_json::from_str::<String>(quoted)?.len(),
                "{quoted}"
            );
        }
        Ok(())
    }

    #[test]
    fn validated_cursor_borrows_nested_values_and_reuses_escaped_key_scratch(
    ) -> serde_json::Result<()> {
        let encoded = r#""\ud83d\ude42\b\f\n\r\t\/\\\"\u0069é""#;
        let value = r#"{"nested":[{},{"str":"},{\"[]\\\""}]}"#;
        let expected: String = serde_json::from_str(encoded)?;
        let mut scratch = String::new();
        assert_eq!(validated_key(encoded, &mut scratch)?, expected);
        let capacity = scratch.capacity();
        let input = format!(
            r#"{{{}"id":7}}"#,
            format!("{encoded}:{value},").repeat(4096)
        );
        super::super::validate_json_inner(&input)?;
        let mut cursor = &input[1..];
        for _ in 0..4096 {
            let key = take_validated(&mut cursor)?;
            assert_eq!(validated_key(key, &mut scratch)?, expected);
            assert_eq!(scratch.capacity(), capacity);
            cursor = &cursor[1..]; // colon
            let start = cursor.as_ptr();
            let leaf = take_validated(&mut cursor)?;
            assert_eq!(leaf, value);
            assert_eq!(leaf.as_ptr(), start);
            cursor = &cursor[1..]; // comma
        }
        assert_eq!(cursor, r#""id":7}"#);
        Ok(())
    }
}
