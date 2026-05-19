// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Shared FAST host-attribute propagation helper.
//!
//! This module is internal to the FAST parser plugins (`fast_v2` and
//! `fast_v3`). It owns the FAST-specific policy for deciding which
//! template host attributes to propagate onto a custom element opening
//! tag at SSR time. The WebUI parser core remains framework-neutral —
//! it only provides the generic `on_template_root_attributes` and
//! `host_element_attributes` plugin hooks; FAST plugins delegate to this
//! helper to implement those hooks.
//!
//! Policy enforced here:
//! * Static attributes only. Any attribute whose source text contains
//!   handlebars (`{{`) is skipped — dynamic propagation is out of scope.
//! * Names starting with `@`, `:`, or `?` are skipped (FAST runtime
//!   bindings).
//! * Names `f-ref`, `f-slotted`, `f-children` are skipped (FAST
//!   client-only directives).
//! * Names `shadowrootmode` and `shadowrootadoptedstylesheets` are
//!   skipped (declarative-shadow-DOM-only attributes).
//! * Propagation is gated on `DomStrategy::Shadow`.
//! * Author-provided host attributes win on conflict. Conflict
//!   detection strips leading `:` or `?` from the author name so binding
//!   forms suppress static propagation; `@` event prefixes do not (they
//!   target a different namespace).

use std::collections::HashMap;
use std::collections::HashSet;

use super::TemplateRootAttribute;
use crate::DomStrategy;

/// Per-component FAST host-attribute cache, keyed by component tag name.
///
/// One instance per FAST plugin. Entries are populated by [`Self::capture`]
/// (called from the plugin's `on_template_root_attributes` hook) and read by
/// [`Self::produce_for_host`] (called from the plugin's
/// `host_element_attributes` hook).
#[derive(Debug, Default)]
pub(crate) struct FastHostAttrs {
    dom_strategy: DomStrategy,
    by_tag: HashMap<String, Vec<CapturedAttr>>,
}

#[derive(Debug, Clone)]
struct CapturedAttr {
    /// Lowercased attribute name used for author-conflict comparison.
    normalized_name: String,
    /// Verbatim attribute source text without any leading whitespace
    /// (e.g. `autofocus` or `tabindex="0"`). The parser is responsible
    /// for inserting separator whitespace.
    raw_text: String,
}

impl FastHostAttrs {
    /// Create an empty host-attribute cache. Defaults to Light DOM until
    /// [`Self::set_dom_strategy`] is called.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Configure the DOM strategy. Propagation is only produced when the
    /// strategy is [`DomStrategy::Shadow`].
    pub(crate) fn set_dom_strategy(&mut self, strategy: DomStrategy) {
        self.dom_strategy = strategy;
    }

    /// Filter the generic `<template>` root attributes through the FAST
    /// skip list and cache the propagatable subset keyed by tag name.
    ///
    /// Idempotent: re-capturing an already-cached tag overwrites the
    /// previous entry, which is the desired behavior if the same
    /// component is re-registered.
    pub(crate) fn capture(&mut self, tag_name: &str, attrs: &[TemplateRootAttribute]) {
        let mut captured = Vec::with_capacity(attrs.len());
        for attr in attrs {
            if is_fast_client_only(&attr.name) {
                continue;
            }
            if attr.raw_text.contains("{{") {
                continue;
            }
            captured.push(CapturedAttr {
                normalized_name: attr.name.to_ascii_lowercase(),
                raw_text: attr.raw_text.clone(),
            });
        }
        self.by_tag.insert(tag_name.to_string(), captured);
    }

    /// Produce the host-attribute injection string for a usage-site host
    /// opening tag, or `None` when there is nothing to inject.
    ///
    /// Returns text **without** a leading separator space — the parser
    /// prepends a single space before splicing.
    ///
    /// `author_attr_names` are the attribute names written at the
    /// usage site, in source spelling (e.g. `tabindex`, `?disabled`,
    /// `@click`). They are normalized internally per the FAST conflict
    /// rule: a leading `:` or `?` is stripped, but a leading `@` is not.
    pub(crate) fn produce_for_host(
        &self,
        tag_name: &str,
        author_attr_names: &[&str],
    ) -> Option<String> {
        if self.dom_strategy != DomStrategy::Shadow {
            return None;
        }
        let attrs = self.by_tag.get(tag_name)?;
        if attrs.is_empty() {
            return None;
        }

        let mut author_set: HashSet<String> = HashSet::with_capacity(author_attr_names.len());
        for name in author_attr_names {
            author_set.insert(normalize_author_attr_name_for_conflict(name));
        }

        let total: usize = attrs.iter().map(|a| a.raw_text.len() + 1).sum();
        let mut out = String::with_capacity(total);
        for attr in attrs {
            if author_set.contains(&attr.normalized_name) {
                continue;
            }
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&attr.raw_text);
        }

        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }
}

/// Returns `true` for attribute names that FAST treats as client-only
/// and therefore must never propagate from a component's template
/// wrapper onto the host element at SSR time.
fn is_fast_client_only(name: &str) -> bool {
    if name.is_empty() {
        return true;
    }
    let first = name.as_bytes()[0];
    if first == b'@' || first == b':' || first == b'?' {
        return true;
    }
    matches!(
        name,
        "f-ref" | "f-slotted" | "f-children" | "shadowrootmode" | "shadowrootadoptedstylesheets"
    )
}

/// Normalize an author-written attribute name for conflict comparison.
///
/// FAST binding prefixes `:` (complex property) and `?` (boolean
/// attribute) target the same underlying attribute name, so an author
/// `?disabled` should suppress a static template `disabled`. The `@`
/// prefix denotes an event listener that targets a different namespace
/// than a same-named attribute (e.g. `@click` is the `click` event, not
/// the `click` attribute), so it is **not** stripped.
fn normalize_author_attr_name_for_conflict(name: &str) -> String {
    let stripped = name.strip_prefix(':').or_else(|| name.strip_prefix('?'));
    stripped.unwrap_or(name).to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]

    use super::*;

    fn attr(name: &str, raw: &str) -> TemplateRootAttribute {
        TemplateRootAttribute {
            name: name.to_string(),
            value: None,
            raw_text: raw.to_string(),
        }
    }

    #[test]
    fn is_fast_client_only_recognizes_binding_prefixes() {
        assert!(is_fast_client_only("@click"));
        assert!(is_fast_client_only(":data"));
        assert!(is_fast_client_only("?disabled"));
        assert!(!is_fast_client_only("autofocus"));
    }

    #[test]
    fn is_fast_client_only_recognizes_directive_names() {
        assert!(is_fast_client_only("f-ref"));
        assert!(is_fast_client_only("f-slotted"));
        assert!(is_fast_client_only("f-children"));
        assert!(is_fast_client_only("shadowrootmode"));
        assert!(is_fast_client_only("shadowrootadoptedstylesheets"));
        assert!(!is_fast_client_only("f-template-passthrough"));
    }

    #[test]
    fn normalize_author_attr_strips_binding_prefixes_only() {
        assert_eq!(
            normalize_author_attr_name_for_conflict(":foo"),
            "foo".to_string()
        );
        assert_eq!(
            normalize_author_attr_name_for_conflict("?bar"),
            "bar".to_string()
        );
        // @ is NOT stripped: event handler does not conflict with same-named attr.
        assert_eq!(
            normalize_author_attr_name_for_conflict("@click"),
            "@click".to_string()
        );
        assert_eq!(
            normalize_author_attr_name_for_conflict("AutoFocus"),
            "autofocus".to_string()
        );
    }

    #[test]
    fn produce_for_host_returns_none_when_light_dom() {
        let mut cache = FastHostAttrs::new();
        cache.set_dom_strategy(DomStrategy::Light);
        cache.capture("host-card", &[attr("autofocus", "autofocus")]);
        assert!(cache.produce_for_host("host-card", &[]).is_none());
    }

    #[test]
    fn produce_for_host_concatenates_with_single_space_separator() {
        let mut cache = FastHostAttrs::new();
        cache.set_dom_strategy(DomStrategy::Shadow);
        cache.capture(
            "host-card",
            &[
                attr("autofocus", "autofocus"),
                attr("tabindex", "tabindex=\"0\""),
            ],
        );
        let out = cache
            .produce_for_host("host-card", &[])
            .expect("expected text");
        assert_eq!(out, "autofocus tabindex=\"0\"");
    }

    #[test]
    fn produce_for_host_suppresses_conflicting_author_names() {
        let mut cache = FastHostAttrs::new();
        cache.set_dom_strategy(DomStrategy::Shadow);
        cache.capture(
            "host-card",
            &[
                attr("autofocus", "autofocus"),
                attr("tabindex", "tabindex=\"0\""),
            ],
        );
        let out = cache
            .produce_for_host("host-card", &["tabindex"])
            .expect("autofocus survives");
        assert_eq!(out, "autofocus");
    }

    #[test]
    fn produce_for_host_normalizes_boolean_binding_prefix_for_conflict() {
        let mut cache = FastHostAttrs::new();
        cache.set_dom_strategy(DomStrategy::Shadow);
        cache.capture("host-card", &[attr("autofocus", "autofocus")]);
        assert!(cache
            .produce_for_host("host-card", &["?autofocus"])
            .is_none());
    }

    #[test]
    fn produce_for_host_does_not_normalize_event_prefix() {
        let mut cache = FastHostAttrs::new();
        cache.set_dom_strategy(DomStrategy::Shadow);
        cache.capture("host-card", &[attr("click", "click=\"x\"")]);
        let out = cache
            .produce_for_host("host-card", &["@click"])
            .expect("@click does not suppress click attr");
        assert_eq!(out, "click=\"x\"");
    }

    #[test]
    fn capture_filters_client_only_attributes_and_dynamic_values() {
        let mut cache = FastHostAttrs::new();
        cache.set_dom_strategy(DomStrategy::Shadow);
        cache.capture(
            "host-card",
            &[
                attr("shadowrootmode", "shadowrootmode=\"open\""),
                attr("@click", "@click=\"onClick()\""),
                attr(":data", ":data=\"state\""),
                attr("?disabled", "?disabled=\"x\""),
                attr("f-ref", "f-ref=\"r\""),
                attr("title", "title=\"{{label}}\""),
                attr("autofocus", "autofocus"),
            ],
        );
        let out = cache
            .produce_for_host("host-card", &[])
            .expect("autofocus survives");
        assert_eq!(out, "autofocus");
    }

    #[test]
    fn produce_for_host_returns_none_when_no_capture() {
        let mut cache = FastHostAttrs::new();
        cache.set_dom_strategy(DomStrategy::Shadow);
        assert!(cache.produce_for_host("never-captured", &[]).is_none());
    }

    #[test]
    fn produce_for_host_returns_none_when_all_attrs_suppressed() {
        let mut cache = FastHostAttrs::new();
        cache.set_dom_strategy(DomStrategy::Shadow);
        cache.capture("host-card", &[attr("autofocus", "autofocus")]);
        assert!(cache
            .produce_for_host("host-card", &["autofocus"])
            .is_none());
    }
}
