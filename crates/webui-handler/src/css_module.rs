// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Shared helpers for emitting CSS module definitions as `<script type="importmap">`
//! tags with `data:text/css,…` URIs.
//!
//! Both the SSR inline emission path (`lib.rs::emit_css_module_importmap`) and
//! the SPA partial-navigation path (`route_handler.rs::collect_component_assets`)
//! use this helper to produce a single canonical wire shape, so the client only
//! needs to understand one format.

use std::fmt::Write as _;

use serde_json::Value;

/// Build a complete `<script type="importmap">` tag string that registers a
/// single CSS module under `specifier` via a `data:text/css,…` URI.
///
/// If `nonce` is `Some`, a `nonce="…"` attribute is inserted between `type`
/// and `>` so strict CSP `script-src 'nonce-…'` policies allow the inline
/// script. CSS bytes are percent-encoded so they survive the `data:` URI
/// parser; the importmap JSON is produced via `serde_json` so the specifier
/// and URI value are correctly JSON-escaped.
///
/// Requires browser support for Multiple Import Maps (Chrome 133+); the
/// browser merges each emitted importmap into the document-level resolution
/// table.
#[must_use]
pub fn build_importmap_tag(specifier: &str, css: &str, nonce: Option<&str>) -> String {
    let data_uri = build_data_uri(css);
    let body = build_importmap_json(specifier, data_uri);

    // `<script type="importmap"></script>` is 33 chars; `nonce=""` adds 8 +
    // the value. A few extra bytes avoid a reallocation when the body is
    // small.
    let cap = 40 + body.len() + nonce.map_or(0, |n| n.len() + 9);
    let mut out = String::with_capacity(cap);
    out.push_str("<script type=\"importmap\"");
    if let Some(n) = nonce {
        out.push_str(" nonce=\"");
        out.push_str(n);
        out.push('"');
    }
    out.push('>');
    out.push_str(&body);
    out.push_str("</script>");
    out
}

fn build_data_uri(css: &str) -> String {
    let mut out = String::with_capacity("data:text/css,".len() + css.len());
    out.push_str("data:text/css,");
    percent_encode_css_into(css, &mut out);
    out
}

fn build_importmap_json(specifier: &str, data_uri: String) -> String {
    let mut imports = serde_json::Map::with_capacity(1);
    imports.insert(specifier.to_owned(), Value::String(data_uri));
    let mut root = serde_json::Map::with_capacity(1);
    root.insert("imports".into(), Value::Object(imports));
    Value::Object(root).to_string()
}

// Percent-encode bytes that would mis-parse in a `data:` URI or break out
// of the surrounding `<script type="importmap">` raw-text element:
// `%` (escape), `#` (fragment delimiter), `"`, `<` / `>` (HTML script-data
// terminator + attribute parser), whitespace, and non-ASCII / control bytes.
fn percent_encode_css_into(css: &str, out: &mut String) {
    for b in css.bytes() {
        let needs_encoding = matches!(
            b,
            b'%' | b'#' | b'"' | b'<' | b'>' | b' ' | b'\t' | b'\n' | b'\r'
        ) || !(0x20..0x80).contains(&b);
        if needs_encoding {
            let _ = write!(out, "%{:02X}", b);
        } else {
            out.push(char::from(b));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_importmap_tag_basic() {
        let tag = build_importmap_tag("my-comp", "span{color:blue;}", None);
        assert_eq!(
            tag,
            r#"<script type="importmap">{"imports":{"my-comp":"data:text/css,span{color:blue;}"}}</script>"#
        );
    }

    #[test]
    fn build_importmap_tag_with_nonce() {
        let tag = build_importmap_tag("dash-page", "h1{font-size:2rem}", Some("test-nonce-123"));
        assert_eq!(
            tag,
            r#"<script type="importmap" nonce="test-nonce-123">{"imports":{"dash-page":"data:text/css,h1{font-size:2rem}"}}</script>"#
        );
    }

    #[test]
    fn percent_encoder_escapes_url_unsafe_bytes() {
        let mut out = String::new();
        // %, #, ", <, >, whitespace must all be percent-escaped. Printable
        // ASCII outside that set (including `{`, `}`, `\`) is preserved
        // verbatim; JSON-level escaping of `\` and `"` is handled by
        // serde_json when the URI is embedded inside the importmap object.
        percent_encode_css_into(".a{content:\"\\E000 #x %y\";}", &mut out);
        assert_eq!(out, r#".a{content:%22\E000%20%23x%20%25y%22;}"#);
    }

    #[test]
    fn percent_encoder_escapes_non_ascii_bytes() {
        let mut out = String::new();
        percent_encode_css_into(".a::before{content:\"★\"}", &mut out);
        // ★ is U+2605, UTF-8 bytes E2 98 85.
        assert_eq!(out, ".a::before{content:%22%E2%98%85%22}");
    }

    #[test]
    fn empty_css_produces_empty_data_uri() {
        let tag = build_importmap_tag("empty", "", None);
        assert!(tag.contains(r#""empty":"data:text/css,""#));
    }

    #[test]
    fn json_layer_escapes_backslash_in_css() {
        // CSS escapes like `\E000` survive the percent encoder (the bytes
        // are printable ASCII) but must still be JSON-escaped so the
        // resulting importmap parses. `serde_json` produces `\\E000`.
        let tag = build_importmap_tag("ic", "a::before{content:\"\\E000\"}", None);
        assert!(
            tag.contains(r#""data:text/css,a::before{content:%22\\E000%22}""#),
            "backslash inside the data URI must be JSON-escaped (got: {tag})"
        );
    }

    #[test]
    fn css_with_script_close_tag_cannot_break_out_of_importmap_script() {
        // Regression guard: `<` and `>` must be percent-encoded so CSS
        // content containing `</script>` (or any tag-like sequence) cannot
        // terminate the surrounding `<script type="importmap">` element.
        // The HTML parser tokenizes script bodies in raw-text mode and
        // will treat any literal `</script>` as the end tag regardless of
        // JSON quoting.
        let malicious = r#".a::before{content:"</script><script>alert(1)</script>";}"#;
        let tag = build_importmap_tag("evil", malicious, None);

        // Exactly one `</script>` (the real closing tag) and one `<script`
        // (the real opening tag) may appear; the encoded payload must not
        // contribute any extra tag-like sequences.
        assert_eq!(
            tag.matches("</script>").count(),
            1,
            "only the legitimate closing tag may appear: {tag}"
        );
        assert_eq!(
            tag.matches("<script").count(),
            1,
            "only the legitimate opening tag may appear: {tag}"
        );

        // The body (between the opening `>` and the real closing tag)
        // must not contain any raw `<` or `>`.
        let body_start = tag.find('>').expect("opening tag must terminate") + 1;
        let body_end = tag.rfind("</script>").expect("closing tag must exist");
        let body = &tag[body_start..body_end];
        assert!(
            !body.contains('<') && !body.contains('>'),
            "no raw `<` or `>` may appear inside the importmap body: {body}"
        );
    }
}
