// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Minimal HTML entity encoding for safe embedding of dynamic values.
//!
//! A focused, zero-dependency implementation that covers the six characters
//! needed for XSS prevention in HTML text and attribute contexts.

use std::borrow::Cow;

use crate::{ResponseWriter, Result};

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

#[inline]
fn is_style_end_tag(bytes: &[u8], index: usize) -> bool {
    bytes[index] == b'<'
        && bytes[index + 1] == b'/'
        && bytes[index + 2].eq_ignore_ascii_case(&b's')
        && bytes[index + 3].eq_ignore_ascii_case(&b't')
        && bytes[index + 4].eq_ignore_ascii_case(&b'y')
        && bytes[index + 5].eq_ignore_ascii_case(&b'l')
        && bytes[index + 6].eq_ignore_ascii_case(&b'e')
}

/// Return whether CSS needs escaping before it is written into a `<style>`.
pub(crate) fn style_text_needs_escape(css: &str) -> bool {
    let bytes = css.as_bytes();
    let mut index = 0usize;
    while index + 7 <= bytes.len() {
        if is_style_end_tag(bytes, index) {
            return true;
        }
        index += 1;
    }
    false
}

/// Write CSS into an HTML `<style>` raw-text element without allowing an
/// authored, case-insensitive `</style` sequence to terminate the element.
///
/// Escaping only the slash (`<\/style`) preserves the CSS string value and
/// keeps the ordinary path allocation-free.
pub(crate) fn write_style_text(writer: &mut dyn ResponseWriter, css: &str) -> Result<()> {
    let bytes = css.as_bytes();
    let mut chunk_start = 0usize;
    let mut index = 0usize;
    while index + 7 <= bytes.len() {
        if is_style_end_tag(bytes, index) {
            writer.write(&css[chunk_start..index + 1])?;
            writer.write("\\/")?;
            chunk_start = index + 2;
            index += 7;
        } else {
            index += 1;
        }
    }
    writer.write(&css[chunk_start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct StyleWriter {
        output: String,
        writes: usize,
    }

    impl ResponseWriter for StyleWriter {
        fn write(&mut self, content: &str) -> Result<()> {
            self.output.push_str(content);
            self.writes += 1;
            Ok(())
        }

        fn end(&mut self) -> Result<()> {
            Ok(())
        }
    }

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

    #[test]
    fn style_text_preserves_normal_bytes_in_one_write() {
        let mut writer = StyleWriter::default();
        write_style_text(&mut writer, ".card{content:'normal / style'}").unwrap();
        assert_eq!(writer.output, ".card{content:'normal / style'}");
        assert_eq!(writer.writes, 1);
    }

    #[test]
    fn style_text_escapes_mixed_case_end_tags() {
        let mut writer = StyleWriter::default();
        write_style_text(
            &mut writer,
            ".a{content:'</style>'}.b{content:'</StYlE attr>'}",
        )
        .unwrap();
        assert_eq!(
            writer.output,
            ".a{content:'<\\/style>'}.b{content:'<\\/StYlE attr>'}"
        );
    }

    #[test]
    fn style_text_escape_detection_matches_writer() {
        let cases = [
            ("", false),
            ("</styl", false),
            ("< /style", false),
            ("<\\/style", false),
            ("</style", true),
            ("prefix</STYLE>suffix", true),
            ("a</StYlE attr>b", true),
        ];

        for (css, expected) in cases {
            assert_eq!(
                style_text_needs_escape(css),
                expected,
                "unexpected detection for {css:?}"
            );

            let mut writer = StyleWriter::default();
            write_style_text(&mut writer, css).unwrap();
            assert_eq!(
                writer.output != css,
                expected,
                "writer and detector diverged for {css:?}"
            );
        }
    }
}
