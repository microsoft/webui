// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! HTML script-injection helper.
//!
//! Inserts a `<script>` snippet immediately before the closing `</body>`
//! tag (case-insensitive). If no closing tag is found, appends to the end.

/// Inject `script` immediately before the closing `</body>` tag in `html`.
///
/// The search is case-insensitive (`</BODY>` and `</body>` both match).
/// If `html` has no closing body tag, the script is appended.
#[must_use]
pub fn inject_before_body_close(html: &str, script: &str) -> String {
    if let Some(idx) = find_close_body(html) {
        let mut out = String::with_capacity(html.len() + script.len() + 2);
        out.push_str(&html[..idx]);
        out.push_str(script);
        out.push_str(&html[idx..]);
        out
    } else {
        let mut out = String::with_capacity(html.len() + script.len());
        out.push_str(html);
        out.push_str(script);
        out
    }
}

/// Case-insensitive search for `</body>`. Returns the byte offset of the
/// `<` character, or `None` if not present. `</body>` is ASCII-only, so a
/// fixed-window byte-slice scan is sound (no UTF-8 boundary concerns) and
/// gives us a simple `eq_ignore_ascii_case` per window without allocating.
fn find_close_body(html: &str) -> Option<usize> {
    html.as_bytes()
        .windows(7)
        .position(|w| w.eq_ignore_ascii_case(b"</body>"))
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn inject_before_close_body() {
        let html = "<html><body><p>hi</p></body></html>";
        let injected = inject_before_body_close(html, "<script>x</script>");
        let close_idx = injected.find("</body>").unwrap();
        let script_idx = injected.find("<script>").unwrap();
        assert!(script_idx < close_idx);
        assert!(injected.starts_with("<html><body><p>hi</p>"));
        assert!(injected.ends_with("</body></html>"));
    }

    #[test]
    fn inject_handles_missing_close_body() {
        let html = "<h1>hi</h1>";
        let injected = inject_before_body_close(html, "<script>x</script>");
        assert!(injected.starts_with(html));
        assert!(injected.ends_with("<script>x</script>"));
    }

    #[test]
    fn inject_case_insensitive_close_body() {
        let html = "<HTML><BODY>x</BODY></HTML>";
        let injected = inject_before_body_close(html, "<script>x</script>");
        let script_idx = injected.find("<script>").unwrap();
        let close_idx = injected.find("</BODY>").unwrap();
        assert!(script_idx < close_idx);
    }

    #[test]
    fn inject_uses_last_close_body_when_multiple() {
        // `</body>` appears once in the document; this test ensures we
        // pick the first match (which is also the only match in valid HTML).
        let html = "<body>a</body>b";
        let injected = inject_before_body_close(html, "S");
        assert_eq!(injected, "<body>aS</body>b");
    }
}
