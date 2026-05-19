// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Tiny quote-aware scanners for HTML opening tags.
//!
//! These helpers exist to avoid the cost of a tree-sitter parse when we only
//! need to answer a single localized question about an opening tag (e.g.
//! "where does it end?" or "does it contain any runtime-prefixed attributes?").
//!
//! Each function is a single O(n) byte pass over the opening tag and performs
//! no allocation. They are quote-aware (both `"` and `'`), so a `>` inside an
//! attribute value (e.g. `data-x="a>b"` or `x='a>b'`) is never mistaken for
//! the tag terminator.
//!
//! The runtime-prefix scanner is the fast path that lets
//! [`HtmlParser::strip_runtime_attrs_from_template`] skip tree-sitter entirely
//! when no runtime attributes are present — the common case for most
//! components.

/// Return the byte index of the `>` that closes an HTML opening tag, ignoring
/// any `>` that appears inside a quoted attribute value (single or double
/// quotes). Returns `None` if the tag is unterminated.
pub(crate) fn find_tag_close(input: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut quote: u8 = 0;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else {
            match c {
                b'>' => return Some(i),
                b'"' | b'\'' => quote = c,
                _ => {}
            }
        }
        i += 1;
    }
    None
}

/// Return `true` iff the opening tag contains any attribute name beginning
/// with `@`, `:`, or `?` (the WebUI runtime-only prefixes). The scan stops at
/// the first unquoted `>` so attributes on inner elements never trigger a
/// false positive.
pub(crate) fn opening_tag_has_runtime_prefix(input: &str) -> bool {
    let bytes = input.as_bytes();
    let mut i = 0;
    let mut quote: u8 = 0;
    let mut prev_ws = false;
    while i < bytes.len() {
        let c = bytes[i];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
            i += 1;
            continue;
        }
        match c {
            b'>' => return false,
            b' ' | b'\t' | b'\n' | b'\r' => prev_ws = true,
            b'"' | b'\'' => {
                quote = c;
                prev_ws = false;
            }
            b'@' | b':' | b'?' if prev_ws => return true,
            _ => prev_ws = false,
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_tag_close_simple() {
        assert_eq!(find_tag_close("<br>"), Some(3));
        assert_eq!(find_tag_close("<br/>"), Some(4));
        assert_eq!(find_tag_close("<template>"), Some(9));
    }

    #[test]
    fn find_tag_close_skips_double_quoted_gt() {
        assert_eq!(find_tag_close(r#"<if condition="a > b">"#), Some(21));
        assert_eq!(find_tag_close(r#"<if condition="a >= b">"#), Some(22));
    }

    #[test]
    fn find_tag_close_skips_single_quoted_gt() {
        // Single-quoted attribute values must also be respected, otherwise
        // a `>` inside the value would be mistaken for the tag terminator.
        assert_eq!(find_tag_close(r#"<a x='a>b'>"#), Some(10));
    }

    #[test]
    fn find_tag_close_unterminated_returns_none() {
        assert_eq!(find_tag_close("<unterminated"), None);
        assert_eq!(find_tag_close(r#"<a x="never closed"#), None);
    }

    #[test]
    fn runtime_prefix_detects_event() {
        assert!(opening_tag_has_runtime_prefix("<template @click={fn}>"));
    }

    #[test]
    fn runtime_prefix_detects_bind() {
        assert!(opening_tag_has_runtime_prefix(r#"<template :bar="baz">"#));
    }

    #[test]
    fn runtime_prefix_detects_optional() {
        assert!(opening_tag_has_runtime_prefix(r#"<template ?bool="true">"#));
    }

    #[test]
    fn runtime_prefix_ignores_quoted_prefix_char() {
        // A `:`, `@`, or `?` inside a quoted value is not an attribute prefix.
        assert!(!opening_tag_has_runtime_prefix(
            r#"<template href=":colon">"#
        ));
        assert!(!opening_tag_has_runtime_prefix(
            r#"<template href="mailto:x@y">"#
        ));
        assert!(!opening_tag_has_runtime_prefix(r#"<template href="?q=1">"#));
    }

    #[test]
    fn runtime_prefix_ignores_tag_name() {
        // The tag name itself is not preceded by whitespace inside the tag,
        // so even pathological inputs that happen to share a prefix char
        // would not falsely trigger.
        assert!(!opening_tag_has_runtime_prefix("<template>"));
        assert!(!opening_tag_has_runtime_prefix(r#"<template foo="bar">"#));
    }

    #[test]
    fn runtime_prefix_stops_at_unquoted_gt() {
        // Content inside the element must NOT influence the scan; otherwise
        // a child `<span @click="x">` would force tree-sitter on every
        // template that contains any inline events anywhere in the body.
        assert!(!opening_tag_has_runtime_prefix(
            r#"<template><span @click="x"></span></template>"#
        ));
    }
}
