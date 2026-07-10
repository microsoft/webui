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
//! commented-out or stringified decorators, since those false positives cost at
//! most a few bytes while a false negative would be a correctness bug.

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
fn read_decorated_property(bytes: &[u8], mut i: usize) -> Option<(String, usize)> {
    loop {
        i = skip_whitespace(bytes, i);
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
                let name = core::str::from_utf8(&bytes[start..i]).ok()?;
                return Some((name.to_string(), i));
            }
            _ => return None,
        }
    }
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
}
