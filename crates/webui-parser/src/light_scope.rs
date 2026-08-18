// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Per-component scope markers for Light DOM style isolation.
//!
//! Light DOM has no native style boundary. WebUI builds one at compile time by
//! stamping every element a Light component's template declares with a marker
//! attribute, and qualifying every selector in that component's CSS with the
//! matching `:where([marker])` (see [`crate::css_boundary`]).
//!
//! The marker is `data-wl-<id>`, where `<id>` is a base36 FNV-1a hash of the
//! component's tag name. Hashing rather than counting keeps the marker
//! **order-independent**: components compile lazily in discovery order, so a
//! sequential counter would make the emitted protocol depend on which route was
//! parsed first. Collisions are impossible to ignore silently — the caller
//! registers each derived id and reports a diagnostic if two tags collide.
//!
//! Stamping is presence-only (`data-wl-a1b2c3`, no value) because attribute
//! presence is the cheapest thing Blink can bucket and match, and it never
//! interferes with author classes the way a marker class would.

use crate::html_parser::{Event, Walker};

/// Prefix shared by every generated scope marker.
///
/// Extends the reserved `data-wl` family, so one authoring rule covers both.
pub(crate) const SCOPE_MARKER_PREFIX: &str = "data-wl-";

/// Number of base36 digits in a scope id.
const SCOPE_ID_LEN: usize = 6;

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const BASE36: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// Derive the scope marker attribute name for a component tag.
///
/// Deterministic across builds and independent of compilation order.
pub(crate) fn marker_attribute(tag_name: &str) -> String {
    let mut hash = FNV_OFFSET_BASIS;
    for byte in tag_name.as_bytes() {
        hash ^= u64::from(byte.to_ascii_lowercase());
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    let mut marker = String::with_capacity(SCOPE_MARKER_PREFIX.len() + SCOPE_ID_LEN);
    marker.push_str(SCOPE_MARKER_PREFIX);
    let mut digits = [0u8; SCOPE_ID_LEN];
    for slot in digits.iter_mut().rev() {
        *slot = BASE36[(hash % 36) as usize];
        hash /= 36;
    }
    marker.push_str(std::str::from_utf8(&digits).unwrap_or("000000"));
    marker
}

/// Whether an authored attribute name would collide with the marker family.
pub(crate) fn is_reserved_marker(name: &str) -> bool {
    name.get(..SCOPE_MARKER_PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(SCOPE_MARKER_PREFIX))
}

/// Opening delimiter of a raw (unescaped) binding.
///
/// `HandlebarsParser` treats `{{{expr}}}` as a signal whose value is written to
/// the document as markup rather than as text.
const RAW_BINDING_OPEN: &str = "{{{";

/// Whether a template can render elements the compiler never sees.
///
/// A raw binding interpolates author-supplied markup at render time, so the
/// elements it produces exist in no compiled template and cannot be stamped.
/// Their component keeps the `@scope` enclosure, which resolves the boundary at
/// match time and therefore covers DOM of any origin.
///
/// Deliberately conservative: it tests only for the opening delimiter, so a
/// literal `{{{` in text costs that component the stamped fast path but can
/// never cost it correctness.
pub(crate) fn renders_opaque_html(html: &str) -> bool {
    html.contains(RAW_BINDING_OPEN)
}

/// Stamp `marker` onto every element in a Light component template.
///
/// Returns `None` when the template declares no elements, so callers skip the
/// copy entirely. The walk is iterative with an explicit range stack, matching
/// `analyze_component_dom`; templates nest arbitrarily deep and recursion is
/// not an option.
///
/// Stamping the template *string* rather than the emitted fragment stream is
/// deliberate: the same string is what the client runtime assigns to
/// `innerHTML` when a component is created with `document.createElement`, so
/// one implementation covers SSR, the plugin artifact, and client mounts.
pub(crate) fn stamp_template(source: &str, marker: &str) -> Option<String> {
    let mut insertions: Vec<usize> = Vec::new();
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::with_capacity(8);
    ranges.push(0..source.len());

    while let Some(range) = ranges.pop() {
        for event in Walker::new_range(source, range.start, range.end) {
            let Event::Element(element) = event else {
                continue;
            };
            let name = element.name();
            insertions.push(element.start + 1 + name.len());

            let raw_text = name.eq_ignore_ascii_case("script")
                || name.eq_ignore_ascii_case("style")
                || name.eq_ignore_ascii_case("textarea")
                || name.eq_ignore_ascii_case("title");
            if element.content_end() > element.inner().start && !raw_text {
                ranges.push(element.inner());
            }
        }
    }

    if insertions.is_empty() {
        return None;
    }
    // Nested ranges are walked out of source order; splicing requires ascending.
    insertions.sort_unstable();

    let mut stamped = String::with_capacity(source.len() + insertions.len() * (marker.len() + 1));
    let mut copied = 0usize;
    for at in insertions {
        stamped.push_str(&source[copied..at]);
        stamped.push(' ');
        stamped.push_str(marker);
        copied = at;
    }
    stamped.push_str(&source[copied..]);
    Some(stamped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_a_stable_order_independent_marker() {
        let marker = marker_attribute("my-card");
        assert_eq!(marker, marker_attribute("my-card"));
        assert_eq!(marker, marker_attribute("MY-CARD"));
        assert_ne!(marker, marker_attribute("my-cart"));
        assert!(marker.starts_with(SCOPE_MARKER_PREFIX));
        assert_eq!(marker.len(), SCOPE_MARKER_PREFIX.len() + SCOPE_ID_LEN);
        assert!(marker[SCOPE_MARKER_PREFIX.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit()));
    }

    #[test]
    fn reserves_the_whole_marker_family() {
        assert!(is_reserved_marker("data-wl-a1b2c3"));
        assert!(is_reserved_marker("DATA-WL-X"));
        assert!(!is_reserved_marker("data-wl"));
        assert!(!is_reserved_marker("data-wrapper"));
        assert!(!is_reserved_marker("data-w"));
        assert!(!is_reserved_marker("aaaaaaaé"));
        assert!(!is_reserved_marker("data-wlé"));
    }

    #[test]
    fn detects_only_raw_bindings_as_opaque() {
        assert!(renders_opaque_html("<div>{{{descriptionHtml}}}</div>"));
        assert!(renders_opaque_html("<div>{{{ a.b }}}</div>"));
        assert!(!renders_opaque_html("<div>{{description}}</div>"));
        assert!(!renders_opaque_html("<div class=\"{{cls}}\">x</div>"));
        assert!(!renders_opaque_html("<div>plain</div>"));
    }

    #[test]
    fn stamps_every_element_including_nested_and_void() {
        let stamped = stamp_template("<div class=\"a\"><span>x</span><br></div>", "data-wl-x")
            .expect("stamped");
        assert_eq!(
            stamped,
            "<div data-wl-x class=\"a\"><span data-wl-x>x</span><br data-wl-x></div>"
        );
    }

    #[test]
    fn stamps_self_closing_and_component_hosts() {
        let stamped =
            stamp_template("<my-child a=\"1\"/><img src=\"x\"/>", "data-wl-x").expect("stamped");
        assert_eq!(
            stamped,
            "<my-child data-wl-x a=\"1\"/><img data-wl-x src=\"x\"/>"
        );
    }

    /// Inert elements still participate in structural selectors such as
    /// `style + .content`, so both they and cloned `<template>` content are
    /// stamped.
    #[test]
    fn stamps_template_content_and_inert_elements() {
        let stamped = stamp_template(
            "<template foo=\"bar\"><div>content</div></template><style>.a{color:red}</style>",
            "data-wl-x",
        )
        .expect("stamped");
        assert_eq!(
            stamped,
            "<template data-wl-x foo=\"bar\"><div data-wl-x>content</div></template><style data-wl-x>.a{color:red}</style>"
        );
    }

    #[test]
    fn stamps_raw_text_elements_and_leaves_element_free_templates_alone() {
        assert_eq!(
            stamp_template("<style>.a{color:red}</style>", "data-wl-x"),
            Some("<style data-wl-x>.a{color:red}</style>".to_string())
        );
        assert_eq!(stamp_template("plain text", "data-wl-x"), None);
        assert_eq!(stamp_template("", "data-wl-x"), None);
    }
}
