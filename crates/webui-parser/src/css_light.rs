// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! CSS validation and identifier helpers for unscoped Light DOM.
//!
//! Effective Light components use ordinary authored CSS in their owning CSS
//! tree. Shadow-only selectors are rejected instead of being rewritten into a
//! selector that has different semantics outside a ShadowRoot.

use crate::comment_policy;
use crate::css_scan::{
    block_comment_end, css_escape_end, css_identifier_eq, next_char_boundary, pseudo_name,
    quoted_end,
};
use crate::diagnostic::{codes, Diagnostic};
use crate::{ParserError, Result};

/// Reject selectors whose meaning is limited to a ShadowRoot.
pub(crate) fn validate_global_css(tag_name: &str, source: &str) -> Result<()> {
    let bytes = source.as_bytes();
    let mut index = 0;
    let mut segment_start = 0;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'"' | b'\'' => index = quoted_end(source, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = block_comment_end(source, index);
            }
            b'/' if comment_policy::is_css_line_comment_start(source, index) => {
                index = comment_policy::find_css_line_comment_end(source, index + 2);
            }
            b'\\' => index = css_escape_end(bytes, index, bytes.len()),
            b'(' => {
                paren_depth += 1;
                index += 1;
            }
            b')' => {
                paren_depth = paren_depth.saturating_sub(1);
                index += 1;
            }
            b'[' => {
                bracket_depth += 1;
                index += 1;
            }
            b']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                index += 1;
            }
            b'{' if paren_depth == 0 && bracket_depth == 0 => {
                validate_selector_prelude(tag_name, source, segment_start, index)?;
                segment_start = index + 1;
                index += 1;
            }
            b';' | b'}' if paren_depth == 0 && bracket_depth == 0 => {
                segment_start = index + 1;
                index += 1;
            }
            _ => index = next_char_boundary(source, index),
        }
    }
    Ok(())
}

fn validate_selector_prelude(tag_name: &str, source: &str, start: usize, end: usize) -> Result<()> {
    let bytes = source.as_bytes();
    let mut index = start;
    while index < end {
        match bytes[index] {
            b'"' | b'\'' => index = quoted_end(source, index).min(end),
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = block_comment_end(source, index).min(end);
            }
            b'/' if comment_policy::is_css_line_comment_start(source, index) => {
                index = comment_policy::find_css_line_comment_end(source, index + 2).min(end);
            }
            b'\\' => index = css_escape_end(bytes, index, end),
            b':' => {
                let Some(pseudo) = pseudo_name(source, index) else {
                    index += 1;
                    continue;
                };
                let name = &source[pseudo.name.clone()];
                if css_identifier_eq(name, "host")
                    || css_identifier_eq(name, "host-context")
                    || (pseudo.is_element && css_identifier_eq(name, "slotted"))
                {
                    let selector = if pseudo.is_element {
                        format!("::{name}")
                    } else {
                        format!(":{name}")
                    };
                    return Err(shadow_only_selector_error(
                        tag_name, source, index, &selector,
                    ));
                }
                index = pseudo.name.end.min(end);
            }
            _ => index = next_char_boundary(source, index).min(end),
        }
    }
    Ok(())
}

#[cold]
#[inline(never)]
fn shadow_only_selector_error(
    tag_name: &str,
    source: &str,
    offset: usize,
    selector: &str,
) -> ParserError {
    Diagnostic::error(format!("`{selector}` is only valid in Shadow DOM CSS"))
        .code(codes::UNSUPPORTED_LIGHT_CSS)
        .component(tag_name)
        .at_offset(source, offset)
        .snippet(super::source_line_snippet(source, offset))
        .help(
            "use a normal selector such as the component tag, or wrap the complete component in `<template shadowrootmode=\"open\">`",
        )
        .into()
}

/// Append `value` as one CSS identifier without changing its semantic value.
pub(crate) fn push_css_identifier(output: &mut String, value: &str) {
    for (index, ch) in value.chars().enumerate() {
        if ch.is_ascii_alphabetic() || ch == '-' || ch == '_' || (index > 0 && ch.is_ascii_digit())
        {
            output.push(ch);
        } else {
            push_css_escape(output, ch as u32);
        }
    }
}

fn push_css_escape(output: &mut String, mut value: u32) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut digits = [0u8; 8];
    let mut index = digits.len();
    loop {
        index -= 1;
        digits[index] = HEX[(value & 0x0f) as usize];
        value >>= 4;
        if value == 0 {
            break;
        }
    }
    output.push('\\');
    for digit in &digits[index..] {
        output.push(char::from(*digit));
    }
    output.push(' ');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_global_css() {
        assert!(validate_global_css("my-card", ".card, button:hover { color: red }").is_ok());
    }

    #[test]
    fn rejects_shadow_only_selectors() {
        for selector in [
            ":host",
            ":host(.active)",
            ":host-context(body.dark)",
            "::slotted(*)",
        ] {
            let error = validate_global_css("my-card", &format!("{selector} {{ color: red }}"))
                .expect_err("Shadow-only selector must fail in Light DOM");
            let ParserError::Template(diagnostic) = error else {
                panic!("expected structured CSS diagnostic");
            };
            assert_eq!(diagnostic.error_code(), Some(codes::UNSUPPORTED_LIGHT_CSS));
        }
    }

    #[test]
    fn ignores_shadow_selector_text_in_comments_and_strings() {
        assert!(validate_global_css(
            "my-card",
            r#"/* :host */ // :host
.card::before { content: ":host"; }"#
        )
        .is_ok());
    }

    #[test]
    fn ignores_shadow_selector_text_in_declaration_values() {
        assert!(validate_global_css(
            "my-card",
            ".card { --state:host; background:url(:host); content:var(--x, :host); }"
        )
        .is_ok());
    }

    #[test]
    fn rejects_escaped_shadow_only_selector_names() {
        let error = validate_global_css("my-card", r":ho\73 t { color: red }")
            .expect_err("Escaped :host must fail in Light DOM");
        let ParserError::Template(diagnostic) = error else {
            panic!("expected structured CSS diagnostic");
        };
        assert_eq!(diagnostic.error_code(), Some(codes::UNSUPPORTED_LIGHT_CSS));
    }

    #[test]
    fn escapes_non_identifier_tag_characters() {
        let mut output = String::new();
        push_css_identifier(&mut output, "x-foo.bar");
        assert_eq!(output, r"x-foo\2e bar");
    }
}
