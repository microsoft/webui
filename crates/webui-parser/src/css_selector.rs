// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Selector-level segmentation for compiler-owned CSS transforms.
//!
//! A CSS selector list is a comma-separated list of *complex* selectors, each a
//! chain of *compound* selectors joined by combinators. A compound is the unit
//! that matches a single element (`a.b[c]:hover::before`), which makes it the
//! unit every static CSS transform cares about:
//!
//! - **Scoping** (today) appends `:where([data-wl-<id>])` to each compound so a
//!   rule can only match elements the owning component authored.
//! - **Dead-selector elimination** (future) can drop a rule whose compounds name
//!   a class, id, or tag that the owning component's template never emits. That
//!   analysis is only sound because scoping already bounds a rule to one
//!   template, so the element inventory is finite and known at build time.
//! - **Minification** (future) can rewrite class and attribute names once it can
//!   see each compound's simple selectors in isolation.
//!
//! [`for_each_compound`] is the shared primitive. It is a single iterative pass
//! that allocates nothing and reports byte offsets into the caller's selector,
//! so callers splice rather than rebuild. It deliberately reports only
//! **top-level** compounds: the arguments of `:is()`, `:not()`, and `:has()` are
//! matched against the whole document, not relative to the enclosing rule, so a
//! scoping pass must not descend into them.

use crate::comment_policy;
use crate::css_scan::{
    block_comment_end, css_identifier_eq, next_char_boundary, pseudo_name, quoted_end,
};
use std::ops::Range;

/// Pseudo-elements spelled with a single colon by legacy syntax.
///
/// A qualifier must be inserted *before* a pseudo-element, because a compound
/// ends at its pseudo-element: `.a::before:where([x])` is invalid, while
/// `.a:where([x])::before` is the same selector with the qualifier applied.
const LEGACY_PSEUDO_ELEMENTS: [&str; 4] = ["before", "after", "first-line", "first-letter"];

/// One top-level compound selector inside a selector list.
pub(crate) struct Compound {
    /// Byte offset of the first character of the compound.
    pub(crate) start: usize,
    /// Byte offset where a qualifier may be spliced in.
    ///
    /// This is the end of the compound, or the start of its pseudo-element when
    /// it has one.
    pub(crate) insert_at: usize,
    /// Range of a top-level `:scope` token within the compound, when present.
    ///
    /// A compound anchored to the scoping root is already bound to a known
    /// element and must not be qualified further.
    pub(crate) scope_anchor: Option<Range<usize>>,
    /// Whether the compound references the CSS nesting parent (`&`).
    ///
    /// Nested rules inherit whatever qualifier the parent selector already
    /// carries, so an `&` compound must not be qualified again.
    pub(crate) nesting_parent: bool,
}

/// Accumulates one compound as the scanner walks it.
struct Pending {
    start: usize,
    insert_at: Option<usize>,
    scope_anchor: Option<Range<usize>>,
    nesting_parent: bool,
}

impl Pending {
    fn new(start: usize) -> Self {
        Self {
            start,
            insert_at: None,
            scope_anchor: None,
            nesting_parent: false,
        }
    }
}

/// Visit every top-level compound of `selector`, in source order.
///
/// Offsets are relative to `selector`. Compounds are reported as they close, so
/// a caller splicing text must apply the edits back-to-front or copy forward.
pub(crate) fn for_each_compound(selector: &str, mut visit: impl FnMut(Compound)) {
    let bytes = selector.as_bytes();
    let mut open: Option<Pending> = None;
    let mut index = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;

    while index < bytes.len() {
        match bytes[index] {
            b'"' | b'\'' => index = quoted_end(selector, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = block_comment_end(selector, index);
            }
            b'/' if comment_policy::is_css_line_comment_start(selector, index) => {
                index = comment_policy::find_css_line_comment_end(selector, index + 2);
            }
            b'\\' => {
                open.get_or_insert_with(move || Pending::new(index));
                index = next_char_boundary(selector, (index + 1).min(bytes.len()));
            }
            b'(' => {
                paren_depth += 1;
                index += 1;
            }
            b')' => {
                paren_depth = paren_depth.saturating_sub(1);
                index += 1;
            }
            b'[' if paren_depth == 0 => {
                open.get_or_insert_with(move || Pending::new(index));
                bracket_depth += 1;
                index += 1;
            }
            b']' if paren_depth == 0 => {
                bracket_depth = bracket_depth.saturating_sub(1);
                index += 1;
            }
            _ if paren_depth > 0 || bracket_depth > 0 => {
                index = next_char_boundary(selector, index);
            }
            // A comma ends the complex selector; a combinator ends the compound.
            // Both close whatever compound is open, and `||` is consumed whole so
            // its second `|` is not mistaken for a namespace separator.
            b',' | b'>' | b'+' | b'~' => {
                close(&mut open, index, &mut visit);
                index += 1;
            }
            b'|' if bytes.get(index + 1) == Some(&b'|') => {
                close(&mut open, index, &mut visit);
                index += 2;
            }
            byte if byte.is_ascii_whitespace() => {
                close(&mut open, index, &mut visit);
                index += 1;
            }
            b':' => {
                index = scan_pseudo(
                    selector,
                    index,
                    open.get_or_insert_with(move || Pending::new(index)),
                );
            }
            b'&' => {
                open.get_or_insert_with(move || Pending::new(index))
                    .nesting_parent = true;
                index += 1;
            }
            _ => {
                open.get_or_insert_with(move || Pending::new(index));
                index = next_char_boundary(selector, index);
            }
        }
    }

    close(&mut open, bytes.len(), &mut visit);
}

/// Classify the pseudo token at `start` and return the offset just past it.
fn scan_pseudo(selector: &str, start: usize, pending: &mut Pending) -> usize {
    let Some(pseudo) = pseudo_name(selector, start) else {
        // A lone `:` is not a pseudo token; treat it as opaque compound text.
        return next_char_boundary(selector, start);
    };
    let name = &selector[pseudo.name.clone()];
    let functional = selector.as_bytes().get(pseudo.name.end) == Some(&b'(');

    if pseudo.is_element || (!functional && is_legacy_pseudo_element(name)) {
        pending.insert_at.get_or_insert(start);
    } else if !functional && css_identifier_eq(name, "scope") {
        pending.scope_anchor.get_or_insert(start..pseudo.name.end);
    }
    pseudo.name.end
}

fn is_legacy_pseudo_element(name: &str) -> bool {
    LEGACY_PSEUDO_ELEMENTS
        .iter()
        .any(|candidate| css_identifier_eq(name, candidate))
}

/// Emit the open compound, if any, treating `end` as its exclusive end.
fn close(open: &mut Option<Pending>, end: usize, visit: &mut impl FnMut(Compound)) {
    if let Some(pending) = open.take() {
        visit(Compound {
            start: pending.start,
            insert_at: pending.insert_at.unwrap_or(end),
            scope_anchor: pending.scope_anchor,
            nesting_parent: pending.nesting_parent,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Splice `@` into every non-anchored compound, mirroring how the scoping
    /// pass consumes this module. `@` is not valid selector syntax, so it never
    /// collides with the input under test.
    fn qualify(selector: &str) -> String {
        let mut edits: Vec<usize> = Vec::new();
        for_each_compound(selector, |compound| {
            if compound.scope_anchor.is_none() && !compound.nesting_parent {
                edits.push(compound.insert_at);
            }
        });
        let mut out = String::with_capacity(selector.len() + edits.len());
        let mut copied = 0usize;
        for at in edits {
            out.push_str(&selector[copied..at]);
            out.push('@');
            copied = at;
        }
        out.push_str(&selector[copied..]);
        out
    }

    #[test]
    fn qualifies_every_top_level_compound() {
        assert_eq!(qualify(".a .b"), ".a@ .b@");
        assert_eq!(qualify(".a>.b+.c~.d"), ".a@>.b@+.c@~.d@");
        assert_eq!(qualify(".a, .b"), ".a@, .b@");
        assert_eq!(qualify("div"), "div@");
        assert_eq!(qualify("*"), "*@");
        assert_eq!(qualify("  .a  "), "  .a@  ");
    }

    #[test]
    fn qualifies_before_pseudo_elements_only() {
        assert_eq!(qualify(".a::before"), ".a@::before");
        assert_eq!(qualify(".a:before"), ".a@:before");
        assert_eq!(qualify(".a:hover"), ".a:hover@");
        assert_eq!(qualify(".a:hover::after"), ".a:hover@::after");
        assert_eq!(qualify("li:first-letter"), "li@:first-letter");
        // `:first-child` is a pseudo-class despite the shared prefix.
        assert_eq!(qualify("li:first-child"), "li:first-child@");
    }

    #[test]
    fn never_descends_into_functional_pseudo_classes() {
        assert_eq!(qualify(":is(.a .b)"), ":is(.a .b)@");
        assert_eq!(qualify(".a:not(.b .c)"), ".a:not(.b .c)@");
        assert_eq!(qualify(".a:has(> .b)"), ".a:has(> .b)@");
        assert_eq!(qualify(".a:nth-child(2n + 1)"), ".a:nth-child(2n + 1)@");
        // A nested list still segments outside the parens.
        assert_eq!(qualify(":is(.a, .b) .c"), ":is(.a, .b)@ .c@");
    }

    #[test]
    fn skips_anchored_compounds() {
        assert_eq!(qualify(":scope .a"), ":scope .a@");
        assert_eq!(qualify(":scope:is(.wide) > .a"), ":scope:is(.wide) > .a@");
        assert_eq!(qualify("& .a"), "& .a@");
        assert_eq!(qualify("&:hover"), "&:hover");
        assert_eq!(qualify("&.active .a"), "&.active .a@");
        // `:scope` inside a functional argument is not a top-level anchor.
        assert_eq!(qualify(":is(:scope) .a"), ":is(:scope)@ .a@");
    }

    #[test]
    fn reports_the_scope_anchor_range() {
        let mut anchors = Vec::new();
        for_each_compound(":scope:is(.wide) > .a", |compound| {
            anchors.push(compound.scope_anchor.clone());
        });
        assert_eq!(anchors, vec![Some(0..6), None]);
    }

    #[test]
    fn treats_brackets_strings_and_comments_as_opaque() {
        assert_eq!(qualify("[data-x=\"a b\"]"), "[data-x=\"a b\"]@");
        assert_eq!(qualify("[href^='/a']"), "[href^='/a']@");
        assert_eq!(qualify("[data-x=\"a>b,c\"] .d"), "[data-x=\"a>b,c\"]@ .d@");
        assert_eq!(qualify(".a/* c d */.b"), ".a/* c d */.b@");
        assert_eq!(qualify("[title=':scope']"), "[title=':scope']@");
    }

    #[test]
    fn handles_escapes_namespaces_and_non_ascii() {
        assert_eq!(qualify(".\\.a .b"), ".\\.a@ .b@");
        assert_eq!(qualify("ns|div"), "ns|div@");
        assert_eq!(qualify("a||td"), "a@||td@");
        assert_eq!(
            qualify(".\u{e9} .\u{6807}\u{9898}"),
            ".\u{e9}@ .\u{6807}\u{9898}@"
        );
    }
}
