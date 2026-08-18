// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Byte-level CSS token primitives shared by every compiler-owned CSS pass.
//!
//! This is the bottom layer of the CSS toolkit:
//!
//! 1. `css_scan` (this module) — advance past one token: a string, a comment,
//!    an escape, an identifier, a balanced paren group.
//! 2. [`crate::css_selector`] — segment a selector list into compounds.
//! 3. [`crate::css_boundary`] — the Light DOM scoping transform.
//!
//! Every function here takes a byte offset and returns the offset just past the
//! token, so callers stay iterative and allocation-free. Nothing here allocates
//! except [`identifier_value`], which only does so for escaped or quoted input.

use crate::comment_policy;
use std::ops::Range;

/// Advance past a quoted string starting at `start`.
///
/// Unterminated strings consume the remaining input rather than failing: the
/// caller is a build-time rewriter, and a malformed stylesheet must still make
/// forward progress so the browser (not the compiler) reports it.
pub(crate) fn quoted_end(source: &str, start: usize) -> usize {
    let bytes = source.as_bytes();
    let quote = bytes[start];
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
        } else if bytes[index] == quote {
            return index + 1;
        } else {
            index += 1;
        }
    }
    bytes.len()
}

/// Advance past a `/* … */` comment starting at `start`.
pub(crate) fn block_comment_end(source: &str, start: usize) -> usize {
    source[start + 2..]
        .find("*/")
        .map_or(source.len(), |offset| start + offset + 4)
}

/// Advance one UTF-8 character from `index`.
pub(crate) fn next_char_boundary(source: &str, index: usize) -> usize {
    next_char_boundary_from_bytes(source.as_bytes(), index, source.len())
}

/// Advance one UTF-8 character within a byte slice, clamped to `limit`.
pub(crate) fn next_char_boundary_from_bytes(bytes: &[u8], index: usize, limit: usize) -> usize {
    if index >= limit || bytes[index].is_ascii() {
        return (index + 1).min(limit);
    }
    let width = if bytes[index] & 0b1111_1000 == 0b1111_0000 {
        4
    } else if bytes[index] & 0b1111_0000 == 0b1110_0000 {
        3
    } else {
        2
    };
    (index + width).min(limit)
}

/// Advance past an identifier, following CSS escape sequences.
pub(crate) fn ident_end(bytes: &[u8], mut index: usize, limit: usize) -> usize {
    while index < limit {
        if is_ident_byte(bytes[index]) {
            index += 1;
        } else if bytes[index] == b'\\' {
            index = css_escape_end(bytes, index, limit);
        } else {
            break;
        }
    }
    index
}

/// Whether `byte` can begin a CSS identifier.
pub(crate) fn is_ident_start_byte(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'-' | b'_' | b'\\') || byte >= 0x80
}

/// Whether `byte` can continue a CSS identifier.
pub(crate) fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_') || byte >= 0x80
}

/// Whether an identifier token *starts* at `index` rather than continuing one.
pub(crate) fn is_identifier_token_start(bytes: &[u8], index: usize, start: usize) -> bool {
    is_ident_start_byte(bytes[index])
        && (index == start || !is_ident_byte(bytes[index - 1]) && bytes[index - 1] != b'\\')
}

/// Advance past a CSS escape sequence (`\26`, `\@`, …) starting at `start`.
pub(crate) fn css_escape_end(bytes: &[u8], start: usize, limit: usize) -> usize {
    let mut index = start + 1;
    if index >= limit {
        return index;
    }
    if bytes[index].is_ascii_hexdigit() {
        let mut digits = 0usize;
        while index < limit && digits < 6 && bytes[index].is_ascii_hexdigit() {
            index += 1;
            digits += 1;
        }
        if index < limit && bytes[index].is_ascii_whitespace() {
            if bytes[index] == b'\r' && index + 1 < limit && bytes.get(index + 1) == Some(&b'\n') {
                index += 1;
            }
            index += 1;
        }
        index
    } else {
        next_char_boundary_from_bytes(bytes, index, limit)
    }
}

/// Decode a CSS identifier or string token to its value.
///
/// Returns the token unchanged when it is neither quoted nor escaped, so the
/// common case never allocates beyond the copy the caller asked for.
pub(crate) fn identifier_value(token: &str) -> String {
    let quoted = matches!(token.as_bytes().first(), Some(b'"' | b'\''));
    if !quoted && !token.contains('\\') {
        return token.to_string();
    }
    let bytes = token.as_bytes();
    let mut value = String::with_capacity(token.len().saturating_sub(2 * usize::from(quoted)));
    let mut index = usize::from(quoted);
    let end = token.len().saturating_sub(usize::from(quoted));
    while index < end {
        if bytes[index] != b'\\' {
            let next = next_char_boundary(token, index).min(end);
            value.push_str(&token[index..next]);
            index = next;
            continue;
        }
        index += 1;
        if index >= end {
            break;
        }
        if bytes[index].is_ascii_hexdigit() {
            let mut codepoint = 0u32;
            let mut digits = 0usize;
            while index < end && digits < 6 && bytes[index].is_ascii_hexdigit() {
                codepoint = codepoint * 16 + u32::from(hex_value(bytes[index]));
                index += 1;
                digits += 1;
            }
            if index < end && bytes[index].is_ascii_whitespace() {
                if bytes[index] == b'\r' && index + 1 < end && bytes.get(index + 1) == Some(&b'\n')
                {
                    index += 1;
                }
                index += 1;
            }
            value.push(if codepoint == 0 {
                char::REPLACEMENT_CHARACTER
            } else {
                char::from_u32(codepoint).unwrap_or(char::REPLACEMENT_CHARACTER)
            });
        } else if matches!(bytes[index], b'\n' | b'\r' | b'\x0c') {
            if bytes[index] == b'\r' && index + 1 < end && bytes.get(index + 1) == Some(&b'\n') {
                index += 1;
            }
            index += 1;
        } else {
            let next = next_char_boundary(token, index).min(end);
            value.push_str(&token[index..next]);
            index = next;
        }
    }
    value
}

/// Compare a possibly-escaped CSS identifier against a plain ASCII name.
pub(crate) fn css_identifier_eq(identifier: &str, expected: &str) -> bool {
    if identifier.contains('\\') {
        identifier_value(identifier).eq_ignore_ascii_case(expected)
    } else {
        identifier.eq_ignore_ascii_case(expected)
    }
}

fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => 0,
    }
}

/// A `:pseudo` or `::pseudo` token.
pub(crate) struct PseudoName {
    /// Range of the name, excluding the leading colon(s).
    pub(crate) name: Range<usize>,
    /// True for `::name`, the pseudo-element form.
    pub(crate) is_element: bool,
}

/// Read the pseudo-class or pseudo-element token starting at a `:`.
pub(crate) fn pseudo_name(source: &str, start: usize) -> Option<PseudoName> {
    let bytes = source.as_bytes();
    let is_element = bytes.get(start + 1) == Some(&b':');
    let name_start = start + 1 + usize::from(is_element);
    if !bytes
        .get(name_start)
        .is_some_and(|byte| is_ident_start_byte(*byte))
    {
        return None;
    }

    let name_end = ident_end(bytes, name_start, bytes.len());
    Some(PseudoName {
        name: name_start..name_end,
        is_element,
    })
}

/// Find the `)` that closes the `(` at `open`, skipping nested groups,
/// strings, and comments. Returns the offset just past the `)`.
pub(crate) fn matching_paren_end(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = open + 1;
    let mut depth = 1usize;
    while index < bytes.len() {
        match bytes[index] {
            b'"' | b'\'' => index = quoted_end(source, index),
            b'\\' => index = css_escape_end(bytes, index, bytes.len()),
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = block_comment_end(source, index);
            }
            b'/' if comment_policy::is_css_line_comment_start(source, index) => {
                index = comment_policy::find_css_line_comment_end(source, index + 2);
            }
            b'(' => {
                depth += 1;
                index += 1;
            }
            b')' => {
                depth -= 1;
                index += 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => index = next_char_boundary(source, index),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advances_past_strings_comments_and_escapes() {
        assert_eq!(quoted_end("\"a\\\"b\" tail", 0), 6);
        assert_eq!(quoted_end("'unterminated", 0), 13);
        assert_eq!(block_comment_end("/* c */x", 0), 7);
        assert_eq!(block_comment_end("/* unterminated", 0), 15);
        assert_eq!(css_escape_end(b"\\26 x", 0, 5), 4);
        assert_eq!(css_escape_end(b"\\@rest", 0, 6), 2);
        assert_eq!(matching_paren_end(r"(\)) tail", 0), Some(4));
    }

    #[test]
    fn decodes_escaped_and_quoted_identifiers() {
        assert_eq!(identifier_value("plain"), "plain");
        assert_eq!(identifier_value("\"quoted\""), "quoted");
        assert_eq!(identifier_value("\\68 ost"), "host");
        assert!(css_identifier_eq("\\68 ost", "host"));
        assert!(css_identifier_eq("HOST", "host"));
        assert!(!css_identifier_eq("hosted", "host"));
    }

    #[test]
    fn reads_pseudo_tokens_and_paren_groups() {
        let pseudo = pseudo_name(":hover", 0).expect("pseudo-class");
        assert!(!pseudo.is_element);
        assert_eq!(&":hover"[pseudo.name], "hover");

        let element = pseudo_name("::before", 0).expect("pseudo-element");
        assert!(element.is_element);
        assert_eq!(&"::before"[element.name], "before");

        assert!(pseudo_name(":", 0).is_none());
        assert_eq!(matching_paren_end(":is(a, (b))x", 3), Some(11));
        assert_eq!(matching_paren_end(":is(a", 3), None);
    }
}
