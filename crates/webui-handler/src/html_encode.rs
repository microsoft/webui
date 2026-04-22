// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Minimal HTML entity encoding for safe embedding of dynamic values.
//!
//! A focused, zero-dependency implementation that covers the six characters
//! needed for XSS prevention in HTML text and attribute contexts.

use std::borrow::Cow;

/// Characters escaped and their replacements:
///
/// * `&` → `&amp;`
/// * `<` → `&lt;`
/// * `>` → `&gt;`
/// * `"` → `&quot;`
/// * `'` → `&#x27;`
/// * `/` → `&#x2F;`
///
/// Returns [`Cow::Borrowed`] when the input contains no characters that need
/// escaping (zero-allocation fast path).
pub fn encode_safe(input: &str) -> Cow<'_, str> {
    let bytes = input.as_bytes();

    // Fast path: find the first byte that needs escaping.
    let first = bytes
        .iter()
        .position(|b| matches!(b, b'&' | b'<' | b'>' | b'"' | b'\'' | b'/'));

    let Some(pos) = first else {
        return Cow::Borrowed(input);
    };

    // Slow path: allocate and build the escaped string.
    let mut out = String::with_capacity(input.len() + 6);
    out.push_str(&input[..pos]);

    let mut start = pos;
    for (i, &b) in bytes[pos..].iter().enumerate() {
        let replacement = match b {
            b'&' => "&amp;",
            b'<' => "&lt;",
            b'>' => "&gt;",
            b'"' => "&quot;",
            b'\'' => "&#x27;",
            b'/' => "&#x2F;",
            _ => continue,
        };
        // Flush the unescaped run before this match.
        out.push_str(&input[start..pos + i]);
        out.push_str(replacement);
        start = pos + i + 1;
    }
    // Flush any remaining unescaped tail.
    out.push_str(&input[start..]);

    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_escaping_returns_borrowed() {
        let input = "Hello World 123";
        let result = encode_safe(input);
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result, "Hello World 123");
    }

    #[test]
    fn empty_string_returns_borrowed() {
        let result = encode_safe("");
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result, "");
    }

    #[test]
    fn escapes_ampersand() {
        assert_eq!(encode_safe("a&b"), "a&amp;b");
    }

    #[test]
    fn escapes_less_than() {
        assert_eq!(encode_safe("<script>"), "&lt;script&gt;");
    }

    #[test]
    fn escapes_greater_than() {
        assert_eq!(encode_safe("a>b"), "a&gt;b");
    }

    #[test]
    fn escapes_double_quote() {
        assert_eq!(encode_safe(r#"a"b"#), "a&quot;b");
    }

    #[test]
    fn escapes_single_quote() {
        assert_eq!(encode_safe("a'b"), "a&#x27;b");
    }

    #[test]
    fn escapes_forward_slash() {
        assert_eq!(encode_safe("a/b"), "a&#x2F;b");
    }

    #[test]
    fn escapes_all_special_chars() {
        assert_eq!(encode_safe(r#"&<>"'/"#), "&amp;&lt;&gt;&quot;&#x27;&#x2F;");
    }

    #[test]
    fn preserves_unicode() {
        assert_eq!(encode_safe("こんにちは"), "こんにちは");
        assert!(matches!(encode_safe("こんにちは"), Cow::Borrowed(_)));
    }

    #[test]
    fn mixed_unicode_and_special() {
        assert_eq!(encode_safe("日本語&テスト"), "日本語&amp;テスト");
    }

    #[test]
    fn escapes_at_start() {
        assert_eq!(encode_safe("&start"), "&amp;start");
    }

    #[test]
    fn escapes_at_end() {
        assert_eq!(encode_safe("end&"), "end&amp;");
    }

    #[test]
    fn multiple_consecutive_escapes() {
        assert_eq!(encode_safe("&&"), "&amp;&amp;");
    }

    #[test]
    fn realistic_nonce() {
        // CSP nonces are base64 — no special chars expected.
        let nonce = "YWJjZGVmZ2hpamtsbW5vcA==";
        let result = encode_safe(nonce);
        assert!(matches!(result, Cow::Borrowed(_)));
    }

    #[test]
    fn realistic_attribute_value() {
        assert_eq!(
            encode_safe("John O'Brien & Associates"),
            "John O&#x27;Brien &amp; Associates"
        );
    }

    #[test]
    fn realistic_xss_attempt() {
        assert_eq!(
            encode_safe("<script>alert('xss')</script>"),
            "&lt;script&gt;alert(&#x27;xss&#x27;)&lt;&#x2F;script&gt;"
        );
    }

    #[test]
    fn escapes_all_known_special_chars() {
        // Verify all six special characters are escaped correctly.
        let test_cases = [
            ("", ""),
            ("hello", "hello"),
            ("&", "&amp;"),
            ("<", "&lt;"),
            (">", "&gt;"),
            ("\"", "&quot;"),
            ("'", "&#x27;"),
            ("/", "&#x2F;"),
            ("a&b<c>d\"e'f/g", "a&amp;b&lt;c&gt;d&quot;e&#x27;f&#x2F;g"),
        ];
        for (input, expected) in test_cases {
            assert_eq!(encode_safe(input), expected, "Failed for input: {input:?}");
        }
    }
}
