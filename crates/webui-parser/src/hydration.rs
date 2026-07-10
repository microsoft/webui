// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Build-time extraction of a component's hydration surface from its authored
//! client script — the **WebUI parser plugin's** hydration strategy.
//!
//! WebUI components declare reactive state with `@observable` and `@attr`
//! property decorators. Those properties — and only those — are the fields the
//! client runtime restores from the server-emitted bootstrap state during
//! hydration. Extracting their names at build time lets the handler project the
//! SSR state payload down to the hydratable surface instead of serializing the
//! entire server state, which is the dominant startup CPU cost for large state.
//!
//! This convention is owned by [`crate::plugin::webui::WebUIParserPlugin`],
//! which is the sole caller of [`scan_hydration_attributes`] against the raw
//! `Component::script_source` handed to it by the registry. The function stays
//! `pub` so third-party plugins may reuse the same decorator convention, but the
//! plugin-agnostic registration pipeline never scans — each plugin derives its
//! own hydration keys ([`crate::plugin::ComponentTemplateArtifact::hydration_keys`])
//! from whatever build metadata it chooses.
//!
//! This is a deterministic, allocation-light token scanner — no regular
//! expressions and no full TypeScript parse (the guiding principle is *move the
//! decision to build time*). It reads the `.ts`/`.js` source and collects the
//! property name following each `@observable` / `@attr` decorator.
//!
//! ## Safety bias: over-inclusion is harmless, under-inclusion is not
//!
//! A name that is scanned but never actually hydrated only means an extra key is
//! *retained* in the projected payload if the server happens to provide it — the
//! client ignores unknown keys, so it is harmless. A name that is *missed*,
//! however, would be projected out and silently break that field's hydration.
//! The scanner therefore errs toward matching: it does not attempt to skip
//! commented-out or stringified decorators at the top level, since those false
//! positives cost at most a few bytes while a false negative would be a
//! correctness bug.
//!
//! ## TypeScript / JavaScript surface handled
//!
//! Because a *miss* breaks hydration, the property-reading path is deliberately
//! tolerant of the real authoring shapes the example apps use and the quirks JS
//! permits:
//!
//! - **TS member modifiers** — `@observable public count`, `@attr private
//!   readonly total`. A leading run of `public`/`private`/`protected`/
//!   `readonly`/`static`/`declare`/`override`/`accessor`/`abstract` is skipped
//!   so the *property* name is read, not the modifier. The disambiguation rule
//!   is precise: a modifier keyword is only treated as a modifier when another
//!   identifier follows it; otherwise it is itself the property name (a field
//!   literally named `static` in `@observable static = 5`).
//! - **Comments between decorator and property** — `@observable /* doc */ name`
//!   and `@attr // note\n label`. The reading path skips `//` and `/* */` runs.
//! - **Definite-assignment / type annotations** — `@attr subtotal!: string`.
//!   Only the name is read; the trailing `!`, `:`, type, and initializer are
//!   irrelevant, which is also why optional semicolons never matter.
//! - **Decorator factories and stacked decorators** — `@attr({ attribute:
//!   'x' })\n prop` and `@observable @deprecated prop`.

/// Property-decorator keywords that mark a hydratable reactive field.
const HYDRATION_DECORATORS: [&str; 2] = ["observable", "attr"];

/// Scan authored component script source for `@observable` / `@attr` property
/// names, returning them sorted and deduplicated.
///
/// The returned names are property (not attribute) identifiers, matching the
/// keys the client reads from the bootstrap state bag. Non-ASCII identifiers are
/// preserved. The result is empty when the source declares no reactive surface.
#[must_use]
pub fn scan_hydration_attributes(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut names: Vec<String> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'@' {
            if let Some(keyword_end) = match_decorator(bytes, i + 1) {
                if let Some((name, next)) = read_decorated_property(bytes, keyword_end) {
                    names.push(name);
                    i = next;
                    continue;
                }
            }
        }
        i += 1;
    }
    names.sort_unstable();
    names.dedup();
    names
}

/// If `bytes[start..]` begins with a hydration decorator keyword at a token
/// boundary, return the index just past the keyword. `@attribute` does not match
/// `@attr` because the following character is an identifier character.
fn match_decorator(bytes: &[u8], start: usize) -> Option<usize> {
    for keyword in HYDRATION_DECORATORS {
        let end = start + keyword.len();
        if end <= bytes.len()
            && &bytes[start..end] == keyword.as_bytes()
            && (end == bytes.len() || !is_ident_char(bytes[end]))
        {
            return Some(end);
        }
    }
    None
}

/// From just past a decorator keyword, skip optional `(...)` options and any
/// stacked decorators, then read the decorated property identifier. Returns the
/// property name and the index just past it.
///
/// A leading run of TypeScript member modifiers (`public`, `readonly`, …) is
/// skipped so `@observable public count` yields `count`, not `public`. Comments
/// between the decorator and the property are skipped as well. Both are
/// correctness-critical: a missed property name is projected out of the
/// bootstrap state and silently breaks that field's hydration.
fn read_decorated_property(bytes: &[u8], mut i: usize) -> Option<(String, usize)> {
    loop {
        i = skip_ws_and_comments(bytes, i);
        let &byte = bytes.get(i)?;
        match byte {
            b'(' => i = skip_balanced_parens(bytes, i)?,
            b'@' => {
                // Stacked decorator (e.g. `@observable @deprecated prop`): skip
                // its name and keep searching for the property identifier.
                i = skip_identifier(bytes, i + 1);
            }
            _ if is_ident_start(byte) => {
                let start = i;
                i = skip_identifier(bytes, i);
                let word = core::str::from_utf8(&bytes[start..i]).ok()?;
                // A TS member modifier is only a modifier when another
                // identifier follows it; otherwise the keyword is itself the
                // property name (a field literally named `static`, say). Peek
                // past whitespace/comments to disambiguate.
                if is_ts_member_modifier(word) {
                    let after = skip_ws_and_comments(bytes, i);
                    if bytes.get(after).is_some_and(|&b| is_ident_start(b)) {
                        i = after;
                        continue;
                    }
                }
                return Some((word.to_string(), i));
            }
            _ => return None,
        }
    }
}

/// TypeScript class-member modifiers that may sit between a decorator and the
/// property name. Skipping them is required so the *property* identifier is read
/// rather than the modifier keyword.
fn is_ts_member_modifier(word: &str) -> bool {
    matches!(
        word,
        "public"
            | "private"
            | "protected"
            | "readonly"
            | "static"
            | "declare"
            | "override"
            | "accessor"
            | "abstract"
    )
}

/// Skip a balanced parenthesis group starting at `bytes[start] == b'('`,
/// ignoring parentheses that appear inside string literals. Returns the index
/// just past the closing parenthesis.
fn skip_balanced_parens(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            quote @ (b'\'' | b'"' | b'`') => {
                i = skip_string(bytes, i + 1, quote)?;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Skip to just past the closing `quote`, honoring backslash escapes.
fn skip_string(bytes: &[u8], mut i: usize, quote: u8) -> Option<usize> {
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            byte if byte == quote => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

fn skip_whitespace(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Skip a run of ASCII whitespace and `//` / `/* */` comments. Used only on the
/// property-reading path (after a decorator keyword has matched), where a
/// comment between the decorator and the property must not hide the property
/// name. A lone `/` that starts neither comment form is left in place for the
/// caller to reject. The top-level `@`-scan stays comment-unaware by design
/// (over-inclusion is harmless; see the module docs).
fn skip_ws_and_comments(bytes: &[u8], mut i: usize) -> usize {
    loop {
        i = skip_whitespace(bytes, i);
        match (bytes.get(i), bytes.get(i + 1)) {
            (Some(b'/'), Some(b'/')) => {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            (Some(b'/'), Some(b'*')) => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                // Advance past the closing `*/` (clamped for an unterminated
                // comment so the index never runs past the slice).
                i = (i + 2).min(bytes.len());
            }
            _ => return i,
        }
    }
}

fn skip_identifier(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && is_ident_char(bytes[i]) {
        i += 1;
    }
    i
}

/// Identifier continuation byte. Bytes >= 0x80 are treated as identifier bytes
/// so multi-byte UTF-8 identifiers survive intact (validated on extraction).
fn is_ident_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$' || byte >= 0x80
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$' || byte >= 0x80
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_observable_and_attr_same_line() {
        let src = "@observable count = 0;\n@attr label = 'x';";
        assert_eq!(scan_hydration_attributes(src), vec!["count", "label"]);
    }

    #[test]
    fn extracts_property_not_attribute_name() {
        // The state bag is keyed by property name, never the mapped attribute.
        let src = "@attr({ attribute: 'display-value' }) displayValue = 'Ready';";
        assert_eq!(scan_hydration_attributes(src), vec!["displayValue"]);
    }

    #[test]
    fn handles_boolean_attr_options() {
        let src = "@attr({ mode: 'boolean', attribute: 'is-active' }) isActive = false;";
        assert_eq!(scan_hydration_attributes(src), vec!["isActive"]);
    }

    #[test]
    fn handles_decorator_on_preceding_line() {
        let src = "@observable\n  name = '';";
        assert_eq!(scan_hydration_attributes(src), vec!["name"]);
    }

    #[test]
    fn scans_realistic_component() {
        let src = "import { WebUIElement, attr, observable } from '../src/index.js';\n\
                   export class TestAttr extends WebUIElement {\n\
                   @attr label = 'Status';\n\
                   @attr({ attribute: 'display-value' }) displayValue = 'Ready';\n\
                   @attr({ attribute: 'cta-href' }) ctaHref = '/checkout';\n\
                   @attr({ mode: 'boolean', attribute: 'is-active' }) isActive = false;\n\
                   @observable itemId = '42';\n\
                   @observable tag = 'demo';\n\
                   noop() {}\n\
                   }";
        assert_eq!(
            scan_hydration_attributes(src),
            vec![
                "ctaHref",
                "displayValue",
                "isActive",
                "itemId",
                "label",
                "tag"
            ]
        );
    }

    #[test]
    fn ignores_bare_imports_and_other_decorators() {
        // `attr`/`observable` in the import are not decorators; `@event` is not a
        // hydration decorator.
        let src = "import { attr, observable } from 'x';\n@event('click') onClick() {}";
        assert!(scan_hydration_attributes(src).is_empty());
    }

    #[test]
    fn does_not_match_longer_identifiers() {
        // `@attribute` and `@observableThing` are not `@attr` / `@observable`.
        let src = "@attribute foo = 1;\n@observableThing bar = 2;";
        assert!(scan_hydration_attributes(src).is_empty());
    }

    #[test]
    fn deduplicates_repeated_names() {
        let src = "@observable foo = 1;\n@observable foo = 2;";
        assert_eq!(scan_hydration_attributes(src), vec!["foo"]);
    }

    #[test]
    fn tolerates_parens_inside_option_strings() {
        let src = "@attr({ attribute: 'a)b' }) weird = 1;";
        assert_eq!(scan_hydration_attributes(src), vec!["weird"]);
    }

    #[test]
    fn reads_property_after_stacked_decorator() {
        let src = "@observable @deprecated foo = 1;";
        assert_eq!(scan_hydration_attributes(src), vec!["foo"]);
    }

    #[test]
    fn ignores_scoped_package_specifiers() {
        // `@microsoft/fast-element` must not be read as an `@attr` decorator.
        let src = "import { attr } from '@microsoft/fast-element';";
        assert!(scan_hydration_attributes(src).is_empty());
    }

    #[test]
    fn empty_source_yields_no_names() {
        assert!(scan_hydration_attributes("").is_empty());
    }

    #[test]
    fn skips_ts_access_modifiers() {
        // `public`/`private`/`protected` etc. are modifiers, not the property.
        let src = "@observable public count = 0;\n@attr private label = 'x';";
        assert_eq!(scan_hydration_attributes(src), vec!["count", "label"]);
    }

    #[test]
    fn skips_stacked_ts_modifiers() {
        let src = "@observable protected readonly total = 0;";
        assert_eq!(scan_hydration_attributes(src), vec!["total"]);
    }

    #[test]
    fn modifier_word_is_property_when_no_identifier_follows() {
        // A field literally named `static` — the modifier keyword IS the name
        // because `=` (not another identifier) follows it.
        let src = "@observable static = 5;";
        assert_eq!(scan_hydration_attributes(src), vec!["static"]);
    }

    #[test]
    fn reads_accessor_auto_field() {
        let src = "@observable accessor value = 1;";
        assert_eq!(scan_hydration_attributes(src), vec!["value"]);
    }

    #[test]
    fn skips_block_comment_between_decorator_and_property() {
        let src = "@observable /* doc */ name = '';";
        assert_eq!(scan_hydration_attributes(src), vec!["name"]);
    }

    #[test]
    fn skips_line_comment_between_decorator_and_property() {
        let src = "@attr // trailing note\n  label = 'x';";
        assert_eq!(scan_hydration_attributes(src), vec!["label"]);
    }

    #[test]
    fn skips_comment_between_factory_and_property() {
        let src = "@attr({ attribute: 'a-b' }) /* c */ ab = '';";
        assert_eq!(scan_hydration_attributes(src), vec!["ab"]);
    }

    #[test]
    fn reads_name_despite_definite_assignment_and_type() {
        // Matches the commerce `mp-cart-panel` shape: `@attr subtotal!: string;`.
        // Only the property name is read; `!`, the type, and the missing
        // initializer are irrelevant.
        let src = "@attr subtotal!: string;\n@attr({ attribute: 'cart-open' }) cartOpen!: string;";
        assert_eq!(scan_hydration_attributes(src), vec!["cartOpen", "subtotal"]);
    }

    #[test]
    fn scans_commerce_add_to_cart_shape() {
        // Real `mp-add-to-cart` decorators: factory options on the same line,
        // hyphenated attribute mapping, camelCase property names.
        let src = "@attr handle = '';\n\
                   @attr({ attribute: 'product-title' }) productTitle = '';\n\
                   @attr({ attribute: 'image-url' }) imageUrl = '';\n\
                   @attr({ attribute: 'selected-size' }) selectedSize = '';";
        assert_eq!(
            scan_hydration_attributes(src),
            vec!["handle", "imageUrl", "productTitle", "selectedSize"]
        );
    }

    #[test]
    fn tolerates_semicolonless_fields() {
        // ASI: no trailing semicolons. We read the NAME, so this never mattered,
        // but pin it as a regression guard.
        let src = "@observable a = 1\n@observable b = 2\n@attr c = 3";
        assert_eq!(scan_hydration_attributes(src), vec!["a", "b", "c"]);
    }
}
