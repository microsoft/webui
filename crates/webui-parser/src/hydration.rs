// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Build-time extraction of a component's hydration surface from its authored
//! client script — the **WebUI parser plugin's** hydration strategy.
//!
//! WebUI components declare reactive state with `@observable` and `@attr`
//! property decorators. Those explicit JavaScript-owned properties are the
//! fields the client runtime restores from the server-emitted bootstrap state.
//! Compiled template roots are a separate navigation surface because their
//! initial values already exist in the trusted SSR DOM. Extracting decorator
//! names at build time lets the handler project the SSR state payload down to
//! the hydratable surface instead of serializing the entire server state.
//!
//! This convention is owned by [`crate::plugin::webui::WebUIParserPlugin`],
//! which is the sole caller of [`scan_hydration_attributes`] against the raw
//! `Component::script_source` handed to it by the registry. The function stays
//! `pub` so third-party plugins may reuse the same decorator convention, but the
//! plugin-agnostic registration pipeline never scans — each plugin derives its
//! own hydration keys ([`crate::plugin::ComponentTemplateArtifact::hydration`])
//! from whatever build metadata it chooses.
//!
//! This is a deterministic, allocation-light token scanner — no regular
//! expressions and no full TypeScript parse (the guiding principle is *move the
//! decision to build time*). It reads the `.ts`/`.js` source and collects the
//! property name following each `@observable` / `@attr` decorator.
//!
//! ## Lexical filtering and safety
//!
//! The top-level scan skips comments, quoted strings, template literals, and
//! regular-expression literals before matching decorators. Text such as
//! `// @observable apiKey` must not expand the emitted state allowlist merely
//! because a server state object happens to contain the same key. This scanner
//! is still not a TypeScript parser or a security boundary: hosts must not place
//! secrets in client-facing render state. Template roots are retained
//! separately for partial navigation, where client-created components need
//! values that were not already rendered into SSR DOM.
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
//! - **Accessor keywords** — `@observable get fullName()`, `@attr set label(v)`.
//!   A leading `get`/`set` is skipped under the same "only when an identifier
//!   follows" rule, so the accessor name is read rather than the keyword; a
//!   field named `get` (`@observable get = 5`) still reads as `get`.
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
    let mut can_start_regex = true;
    while i < bytes.len() {
        match bytes[i] {
            byte if byte.is_ascii_whitespace() => i += 1,
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                i = skip_line_comment(bytes, i + 2);
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = skip_block_comment(bytes, i + 2);
            }
            quote @ (b'\'' | b'"') => {
                i = skip_string(bytes, i + 1, quote).unwrap_or(bytes.len());
                can_start_regex = false;
            }
            b'`' => {
                i = skip_template_literal(bytes, i + 1).unwrap_or(bytes.len());
                can_start_regex = false;
            }
            b'/' if can_start_regex => {
                if let Some(next) = skip_regex_literal(bytes, i + 1) {
                    i = next;
                    can_start_regex = false;
                } else {
                    i += 1;
                    can_start_regex = true;
                }
            }
            b'@' => {
                if let Some(keyword_end) = match_decorator(bytes, i + 1) {
                    if let Some((name, next)) = read_decorated_property(bytes, keyword_end) {
                        names.push(name);
                        i = next;
                        can_start_regex = false;
                        continue;
                    }
                }
                i += 1;
                can_start_regex = true;
            }
            byte if is_ident_start(byte) => {
                let start = i;
                i = skip_identifier(bytes, i);
                can_start_regex =
                    core::str::from_utf8(&bytes[start..i]).is_ok_and(keyword_allows_regex);
            }
            byte if byte.is_ascii_digit() => {
                i = skip_number(bytes, i);
                can_start_regex = false;
            }
            byte @ (b'+' | b'-') if bytes.get(i + 1) == Some(&byte) => {
                i += 2;
                // Prefix updates still expect an expression; postfix updates
                // still end one. Preserve the preceding token state.
            }
            byte => {
                i += 1;
                can_start_regex = punctuation_allows_regex(byte);
            }
        }
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
                i = skip_stacked_decorator(bytes, i + 1)?;
            }
            _ if is_ident_start(byte) => {
                let start = i;
                i = skip_identifier(bytes, i);
                let word = core::str::from_utf8(&bytes[start..i]).ok()?;
                // A TS member modifier or `get`/`set` accessor keyword is only a
                // prefix when another identifier follows it; otherwise the keyword
                // is itself the property name (a field literally named `static`,
                // or `get`, say). Peek past whitespace/comments to disambiguate.
                if is_ts_member_modifier(word) || is_accessor_keyword(word) {
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

fn skip_stacked_decorator(bytes: &[u8], mut i: usize) -> Option<usize> {
    let &first = bytes.get(i)?;
    if !is_ident_start(first) {
        return None;
    }
    i = skip_identifier(bytes, i);

    loop {
        let dot = skip_ws_and_comments(bytes, i);
        if bytes.get(dot) != Some(&b'.') {
            return Some(i);
        }

        i = skip_ws_and_comments(bytes, dot + 1);
        let &byte = bytes.get(i)?;
        if !is_ident_start(byte) {
            return None;
        }
        i = skip_identifier(bytes, i);
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

/// Accessor keywords that may precede the property name on a decorated accessor
/// (`@observable get fullName()`). Like member modifiers they are skipped only
/// when another identifier follows, so a field literally named `get` or `set`
/// (`@observable get = 5`) still reads the keyword itself as the name. Reading
/// the accessor name rather than the keyword keeps the scanner from *missing*
/// the hydratable field — the under-inclusion the module forbids.
fn is_accessor_keyword(word: &str) -> bool {
    matches!(word, "get" | "set")
}

/// Skip a balanced parenthesis group starting at `bytes[start] == b'('`,
/// ignoring parentheses that appear inside string literals. Returns the index
/// just past the closing parenthesis.
fn skip_balanced_parens(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = start;
    let mut can_start_regex = true;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => {
                depth += 1;
                can_start_regex = true;
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
                can_start_regex = false;
            }
            quote @ (b'\'' | b'"') => {
                i = skip_string(bytes, i + 1, quote)?;
                can_start_regex = false;
                continue;
            }
            b'`' => {
                i = skip_template_literal(bytes, i + 1)?;
                can_start_regex = false;
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                i = skip_line_comment(bytes, i + 2);
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = skip_block_comment(bytes, i + 2);
                continue;
            }
            b'/' if can_start_regex => {
                i = skip_regex_literal(bytes, i + 1)?;
                can_start_regex = false;
                continue;
            }
            byte if is_ident_start(byte) => {
                let word_start = i;
                i = skip_identifier(bytes, i);
                can_start_regex =
                    core::str::from_utf8(&bytes[word_start..i]).is_ok_and(keyword_allows_regex);
                continue;
            }
            byte if byte.is_ascii_digit() => {
                i = skip_number(bytes, i);
                can_start_regex = false;
                continue;
            }
            byte @ (b'+' | b'-') if bytes.get(i + 1) == Some(&byte) => {
                i += 2;
                continue;
            }
            byte => can_start_regex = punctuation_allows_regex(byte),
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

/// Skip a template literal starting just after its opening backtick.
///
/// The common no-interpolation path is allocation-free. Template expressions
/// use an explicit mode stack so nested template literals remain iterative.
fn skip_template_literal(bytes: &[u8], mut i: usize) -> Option<usize> {
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i = (i + 2).min(bytes.len()),
            b'`' => return Some(i + 1),
            b'$' if bytes.get(i + 1) == Some(&b'{') => {
                return skip_template_with_expressions(bytes, i + 2);
            }
            _ => i += 1,
        }
    }
    None
}

#[derive(Clone, Copy)]
enum TemplateMode {
    Text,
    Expression {
        brace_depth: usize,
        can_start_regex: bool,
    },
}

fn skip_template_with_expressions(bytes: &[u8], mut i: usize) -> Option<usize> {
    let mut modes = Vec::with_capacity(4);
    modes.push(TemplateMode::Text);
    modes.push(TemplateMode::Expression {
        brace_depth: 1,
        can_start_regex: true,
    });

    while i < bytes.len() {
        match modes.last().copied()? {
            TemplateMode::Text => {
                i = scan_template_text(bytes, i, &mut modes)?;
                if modes.is_empty() {
                    return Some(i);
                }
            }
            TemplateMode::Expression {
                brace_depth,
                can_start_regex,
            } => {
                i = scan_template_expression(bytes, i, brace_depth, can_start_regex, &mut modes)?;
            }
        }
    }
    None
}

fn scan_template_text(bytes: &[u8], i: usize, modes: &mut Vec<TemplateMode>) -> Option<usize> {
    match bytes[i] {
        b'\\' => Some((i + 2).min(bytes.len())),
        b'`' => {
            modes.pop();
            set_expression_regex_state(modes, false);
            Some(i + 1)
        }
        b'$' if bytes.get(i + 1) == Some(&b'{') => {
            modes.push(TemplateMode::Expression {
                brace_depth: 1,
                can_start_regex: true,
            });
            Some(i + 2)
        }
        _ => Some(i + 1),
    }
}

fn scan_template_expression(
    bytes: &[u8],
    i: usize,
    brace_depth: usize,
    can_start_regex: bool,
    modes: &mut Vec<TemplateMode>,
) -> Option<usize> {
    let byte = bytes[i];
    let next = match byte {
        byte if byte.is_ascii_whitespace() => i + 1,
        b'/' if bytes.get(i + 1) == Some(&b'/') => skip_line_comment(bytes, i + 2),
        b'/' if bytes.get(i + 1) == Some(&b'*') => skip_block_comment(bytes, i + 2),
        quote @ (b'\'' | b'"') => {
            let next = skip_string(bytes, i + 1, quote)?;
            set_expression_regex_state(modes, false);
            next
        }
        b'`' => {
            modes.push(TemplateMode::Text);
            i + 1
        }
        b'{' => {
            set_expression_depth(modes, brace_depth + 1, true);
            i + 1
        }
        b'}' if brace_depth == 1 => {
            modes.pop();
            i + 1
        }
        b'}' => {
            set_expression_depth(modes, brace_depth - 1, false);
            i + 1
        }
        b'/' if can_start_regex => {
            if let Some(next) = skip_regex_literal(bytes, i + 1) {
                set_expression_regex_state(modes, false);
                next
            } else {
                set_expression_regex_state(modes, true);
                i + 1
            }
        }
        byte if is_ident_start(byte) => {
            let next = skip_identifier(bytes, i);
            let allows_regex =
                core::str::from_utf8(&bytes[i..next]).is_ok_and(keyword_allows_regex);
            set_expression_regex_state(modes, allows_regex);
            next
        }
        byte if byte.is_ascii_digit() => {
            let next = skip_number(bytes, i);
            set_expression_regex_state(modes, false);
            next
        }
        byte @ (b'+' | b'-') if bytes.get(i + 1) == Some(&byte) => {
            set_expression_regex_state(modes, can_start_regex);
            i + 2
        }
        byte => {
            set_expression_regex_state(modes, punctuation_allows_regex(byte));
            i + 1
        }
    };
    Some(next)
}

fn set_expression_depth(modes: &mut [TemplateMode], depth: usize, can_start_regex: bool) {
    if let Some(mode @ TemplateMode::Expression { .. }) = modes.last_mut() {
        *mode = TemplateMode::Expression {
            brace_depth: depth,
            can_start_regex,
        };
    }
}

fn set_expression_regex_state(modes: &mut [TemplateMode], can_start_regex: bool) {
    if let Some(TemplateMode::Expression {
        can_start_regex: state,
        ..
    }) = modes.last_mut()
    {
        *state = can_start_regex;
    }
}

fn skip_line_comment(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i] != b'\n' {
        i += 1;
    }
    i
}

fn skip_block_comment(bytes: &[u8], mut i: usize) -> usize {
    while i + 1 < bytes.len() {
        if bytes[i] == b'*' && bytes[i + 1] == b'/' {
            return i + 2;
        }
        i += 1;
    }
    bytes.len()
}

fn skip_regex_literal(bytes: &[u8], mut i: usize) -> Option<usize> {
    let mut in_class = false;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i = (i + 2).min(bytes.len()),
            b'[' => {
                in_class = true;
                i += 1;
            }
            b']' => {
                in_class = false;
                i += 1;
            }
            b'/' if !in_class => {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                return Some(i);
            }
            b'\n' | b'\r' => return None,
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
/// caller to reject.
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

fn skip_number(bytes: &[u8], mut i: usize) -> usize {
    if bytes.get(i) == Some(&b'0') {
        if let Some(radix) = bytes.get(i + 1).copied() {
            if matches!(radix, b'x' | b'X' | b'b' | b'B' | b'o' | b'O') {
                i += 2;
                while i < bytes.len()
                    && (bytes[i] == b'_'
                        || match radix {
                            b'x' | b'X' => bytes[i].is_ascii_hexdigit(),
                            b'b' | b'B' => matches!(bytes[i], b'0' | b'1'),
                            b'o' | b'O' => matches!(bytes[i], b'0'..=b'7'),
                            _ => false,
                        })
                {
                    i += 1;
                }
                if bytes.get(i) == Some(&b'n') {
                    i += 1;
                }
                return i;
            }
        }
    }

    while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
        i += 1;
    }
    skip_decimal_number_suffix(bytes, i)
}

fn skip_decimal_number_suffix(bytes: &[u8], mut i: usize) -> usize {
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
            i += 1;
        }
    }

    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
            i += 1;
        }
    } else if bytes.get(i) == Some(&b'n') {
        i += 1;
    }
    i
}

fn keyword_allows_regex(word: &str) -> bool {
    matches!(
        word,
        "await"
            | "case"
            | "delete"
            | "do"
            | "else"
            | "in"
            | "instanceof"
            | "new"
            | "of"
            | "return"
            | "throw"
            | "typeof"
            | "void"
            | "yield"
    )
}

fn punctuation_allows_regex(byte: u8) -> bool {
    !matches!(byte, b')' | b']' | b'}' | b'.')
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
    fn reads_property_after_qualified_stacked_decorator() {
        let src = "@observable @validate.required(/\\)/) value = 1;";
        assert_eq!(scan_hydration_attributes(src), vec!["value"]);
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
    fn reads_name_after_get_accessor_keyword() {
        // `@observable get fullName()` must yield `fullName`, not `get` — reading
        // the keyword would MISS the hydratable field (under-inclusion).
        let src = "@observable get fullName() { return this._n; }";
        assert_eq!(scan_hydration_attributes(src), vec!["fullName"]);
    }

    #[test]
    fn reads_name_after_set_accessor_keyword() {
        let src = "@attr set label(v) { this._l = v; }";
        assert_eq!(scan_hydration_attributes(src), vec!["label"]);
    }

    #[test]
    fn tolerates_regex_parentheses_in_stacked_decorator_options() {
        let src = "@observable @validate(/\\)/) value = 1;";
        assert_eq!(scan_hydration_attributes(src), vec!["value"]);
    }

    #[test]
    fn get_is_property_name_when_no_identifier_follows() {
        // A field literally named `get` — the keyword IS the name because `=`
        // (not another identifier) follows it.
        let src = "@observable get = 5;";
        assert_eq!(scan_hydration_attributes(src), vec!["get"]);
    }

    #[test]
    fn reads_name_after_modifier_and_accessor_keyword() {
        let src = "@observable public get total() { return 0; }";
        assert_eq!(scan_hydration_attributes(src), vec!["total"]);
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

    #[test]
    fn ignores_decorators_inside_comments() {
        let src = "// @observable lineSecret = '';\n\
                   /* @attr blockSecret = ''; */\n\
                   @observable visible = 1;";
        assert_eq!(scan_hydration_attributes(src), vec!["visible"]);
    }

    #[test]
    fn ignores_decorators_inside_strings_and_templates() {
        let src = "const a = \"@observable doubleSecret = 1\";\n\
                   const b = '@attr singleSecret = 1';\n\
                   const c = `@observable templateSecret = ${\"@attr nestedSecret\"}`;\n\
                   @attr visible = '';";
        assert_eq!(scan_hydration_attributes(src), vec!["visible"]);
    }

    #[test]
    fn ignores_decorators_inside_regex_literals() {
        let src = "const a = /@observable regexSecret/;\n\
                   const b = /[@attr blockSecret]/gi;\n\
                   @observable visible = 1;";
        assert_eq!(scan_hydration_attributes(src), vec!["visible"]);
    }

    #[test]
    fn scans_decorators_after_postfix_update_and_division() {
        let src = "class Demo {\n\
                   x = a++ / b;\n\
                   @observable plusValue = /c/;\n\
                   y = a-- / b;\n\
                   @attr minusValue = /d/;\n\
                   }";
        assert_eq!(
            scan_hydration_attributes(src),
            vec!["minusValue", "plusValue"]
        );
    }

    #[test]
    fn numeric_operators_do_not_corrupt_template_regex_context() {
        let src = "const text = `${1+/{/.test(\"{\")}`;\n\
                   @observable visible = 1;";
        assert_eq!(scan_hydration_attributes(src), vec!["visible"]);
    }

    #[test]
    fn resumes_after_template_interpolation() {
        let src = "const text = `value ${condition ? `nested ${value}` : \"fallback\"}`;\n\
                   @observable visible = 1;";
        assert_eq!(scan_hydration_attributes(src), vec!["visible"]);
    }
}
