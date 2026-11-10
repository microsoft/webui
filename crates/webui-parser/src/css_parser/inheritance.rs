// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{skip_css_trivia, skip_css_trivia_with_whitespace, CssRequirementScope, CssRuleScope};
use crate::css_scan;
use std::collections::{HashMap, HashSet};
use webui_tokens::CssFallbackChain;

#[derive(Default)]
pub(super) struct CssInheritance<'a> {
    definitions: HashSet<(usize, &'a str)>,
    // Flat per-selector rule lists avoid per-branch allocations and selector/property products.
    selectors: HashMap<&'a str, usize>,
    selector_rules: Vec<(usize, Option<usize>)>,
}

impl<'a> CssInheritance<'a> {
    pub(super) fn define(&mut self, source: &'a str, scope: &mut CssRuleScope, name: &'a str) {
        let id = scope.id;
        if *scope
            .local_selector
            .get_or_insert_with(|| self.register(source, id))
        {
            self.definitions.insert((id, name));
        }
    }

    fn register(&mut self, source: &'a str, scope_id: usize) -> bool {
        let Some(selector) = scope_id
            .checked_sub(1)
            .and_then(|start| source.get(start..))
        else {
            return false;
        };
        if !simple_selector_list(selector, |_| {}) {
            return false;
        }
        simple_selector_list(selector, |branch| {
            let previous = self.selectors.insert(branch, self.selector_rules.len());
            self.selector_rules.push((scope_id, previous));
        });
        true
    }

    pub(super) fn resolve(
        &self,
        source: &str,
        local_definitions: &HashSet<&str>,
        requirements: &mut Vec<CssFallbackChain>,
        scopes: &mut Vec<CssRequirementScope>,
    ) {
        if self.definitions.is_empty() {
            return;
        }
        let mut kept = 0;
        for index in 0..requirements.len() {
            let usage = &scopes[index];
            let selector = usage
                .rule_id
                .checked_sub(1)
                .and_then(|start| source.get(start..));
            if let Some(selector) = selector.filter(|_| !usage.in_keyframes) {
                requirements[index].tokens.retain(|name| {
                    !local_definitions.contains(name.as_str())
                        || !self.covers(selector, name.as_str())
                });
            }
            if !requirements[index].tokens.is_empty() {
                requirements.swap(kept, index);
                scopes.swap(kept, index);
                kept += 1;
            }
        }
        requirements.truncate(kept);
        scopes.truncate(kept);
    }

    fn covers(&self, mut selector: &str, name: &str) -> bool {
        let mut inherited = false;
        selector = skip_css_trivia(selector);
        loop {
            let Some((rest, subject)) = self.compound(selector, name) else {
                return false;
            };
            let (tail, whitespace) = skip_css_trivia_with_whitespace(rest);
            match tail.as_bytes().first() {
                Some(b'{') => return inherited || subject,
                Some(b',') if inherited || subject => inherited = false,
                Some(b'>') => inherited |= subject,
                // Siblings retain an earlier ancestor, but not the previous subject.
                Some(b'+' | b'~') => {}
                _ if whitespace => {
                    inherited |= subject;
                    selector = tail;
                    continue;
                }
                _ => return false,
            }
            selector = skip_css_trivia(&tail[1..]);
        }
    }

    fn compound<'s>(&self, selector: &'s str, name: &str) -> Option<(&'s str, bool)> {
        let mut end = simple_selector_end(selector);
        if end == 0 {
            return None;
        }
        let mut covered = self.defines(&selector[..end], name);
        while matches!(selector.as_bytes().get(end), Some(b'.' | b'#')) {
            let next = simple_selector_end(&selector[end..]);
            if next == 0 {
                return None;
            }
            covered |= self.defines(&selector[end..end + next], name);
            end += next;
        }
        Some((&selector[end..], covered))
    }

    fn defines(&self, selector: &str, name: &str) -> bool {
        let mut next = self.selectors.get(selector).copied();
        while let Some(index) = next {
            let (id, previous) = self.selector_rules[index];
            if self.definitions.contains(&(id, name)) {
                return true;
            }
            next = previous;
        }
        false
    }
}

pub(super) fn has_local_default(source: &str, name_end: usize) -> bool {
    let after_name = skip_css_trivia(&source[name_end..]);
    let value = skip_css_trivia(&after_name[1..]);
    let end = css_scan::ident_end(value.as_bytes(), 0, value.len());
    let css_wide = ["inherit", "unset", "initial", "revert", "revert-layer"]
        .iter()
        .any(|keyword| css_scan::css_identifier_eq(&value[..end], keyword));
    if !css_wide {
        return true;
    }
    let mut tail = skip_css_trivia(&value[end..]);
    if let Some(priority) = tail.strip_prefix('!') {
        let priority = skip_css_trivia(priority);
        let end = css_scan::ident_end(priority.as_bytes(), 0, priority.len());
        if !css_scan::css_identifier_eq(&priority[..end], "important") {
            return true;
        }
        tail = skip_css_trivia(&priority[end..]);
    }
    !matches!(tail.as_bytes().first(), None | Some(b';' | b'}'))
}

fn simple_selector_end(selector: &str) -> usize {
    let bytes = selector.as_bytes();
    let start = usize::from(matches!(bytes.first(), Some(b'.' | b'#')));
    if !bytes
        .get(start)
        .is_some_and(|byte| css_scan::is_ident_start_byte(*byte))
    {
        return 0;
    }
    css_scan::ident_end(bytes, start, bytes.len())
}

fn simple_selector_list<'a>(mut selector: &'a str, mut visit: impl FnMut(&'a str)) -> bool {
    loop {
        selector = skip_css_trivia(selector);
        let end = simple_selector_end(selector);
        if end == 0 {
            return false;
        }
        let tail = skip_css_trivia(&selector[end..]);
        visit(&selector[..end]);
        if tail.starts_with('{') {
            return true;
        }
        if !tail.starts_with(',') {
            return false;
        }
        selector = &tail[1..];
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::CssInheritance;
    use crate::css_parser::CssRuleScope;
    use crate::{CssParser, LegalComments};
    use std::collections::HashSet;
    use std::fmt::Write;

    #[test]
    fn css_wide_values_do_not_supply_selector_local_defaults() {
        for value in [
            "inherit",
            "unset",
            "initial",
            "revert",
            "revert-layer",
            " /* comment */ InHeRiT !important",
            "\\69 nherit",
            "unset/**/! important",
        ] {
            let css =
                format!(".scope {{ --brand: {value}; }} .scope span {{ color: var(--brand); }}");
            assert_eq!(
                CssParser::new().extract_tokens(&css).unwrap(),
                HashSet::from(["brand".to_string()]),
                "{value}",
            );
        }
        for value in ["\"inherit\"", "inherit red", "inherit()", "inherit-red"] {
            let css =
                format!(".scope {{ --brand: {value}; }} .scope span {{ color: var(--brand); }}");
            assert!(
                CssParser::new().extract_tokens(&css).unwrap().is_empty(),
                "{value}"
            );
        }
    }

    #[test]
    fn escaped_delimiters_do_not_change_selector_boundaries() {
        for escaped in ["\\;", "\\{", "\\}", "\\[", "\\]", "\\(", "\\)", "\\\""] {
            let css = format!(
                ".scope{escaped}div {{ --local: red; }} div span {{ color: var(--local); }}"
            );
            assert_eq!(
                CssParser::new().extract_tokens(&css).unwrap(),
                HashSet::from(["local".to_string()]),
                "{css}",
            );
            let covered = format!(".scope{escaped}div {{ --local: red; }} .scope{escaped}div span {{ color: var(--local); }}");
            assert!(
                CssParser::new()
                    .extract_tokens(&covered)
                    .unwrap()
                    .is_empty(),
                "{covered}"
            );
        }
    }

    #[test]
    fn keyframe_offsets_are_not_element_selectors() {
        for name in ["@keyframes", "@-webkit-keyframes", "@\\6b eyframes"] {
            for step in ["from", "to", "from, to", "0%, from"] {
                let css = format!("from, to {{ --accent: red; }} {name} pulse {{ {step} {{ color: var(--accent); }} }} .consumer {{ animation: pulse 1s; }}");
                assert_eq!(
                    CssParser::new().extract_tokens(&css).unwrap(),
                    HashSet::from(["accent".to_string()]),
                    "{css}",
                );
            }
        }
    }

    #[test]
    fn selector_comments_preserve_only_real_descendant_whitespace() {
        for selector in [
            ".scope/**/ span",
            ".scope/**/\tspan",
            ".scope/* x */ /* y */span",
        ] {
            let css = format!(".scope {{ --local: red; }} {selector} {{ color: var(--local); }}");
            assert!(
                CssParser::new().extract_tokens(&css).unwrap().is_empty(),
                "{css}"
            );
        }
        for selector in [".scope/**/span", ".scope/* whitespace in comment */span"] {
            let css = format!(".scope {{ --local: red; }} {selector} {{ color: var(--local); }}");
            assert_eq!(
                CssParser::new().extract_tokens(&css).unwrap(),
                HashSet::from(["local".to_string()]),
                "{css}",
            );
        }
    }

    #[test]
    fn selector_property_index_grows_linearly() {
        const COUNT: usize = 256;
        let mut css = String::new();
        for index in 0..COUNT {
            if index != 0 {
                css.push(',');
            }
            write!(css, ".scope-{index}").unwrap();
        }
        css.push_str(" {");
        for index in 0..COUNT {
            write!(css, "--local-{index}: red;").unwrap();
        }
        css.push('}');
        let mut inheritance = CssInheritance::default();
        let mut scope = CssRuleScope {
            id: 1,
            ..CssRuleScope::default()
        };
        let names: Vec<String> = (0..COUNT).map(|index| format!("local-{index}")).collect();
        for name in &names {
            inheritance.define(&css, &mut scope, name);
        }
        assert_eq!(inheritance.definitions.len(), COUNT);
        assert_eq!(inheritance.selectors.len(), COUNT);
        assert_eq!(inheritance.selector_rules.len(), COUNT);
    }

    #[test]
    fn simple_defaults_cover_proven_subjects_and_ancestors() {
        for css in [
            "local-token { --local: red; } local-token span { color: var(--local); }",
            "local-token span { color: var(--local); } local-token { --local: red; }",
            ".scope { --local: red; } .scope { color: var(--local); }",
            ".scope { --other: blue; } .scope { --local: red; } .scope span { color: var(--local); background: var(--other); }",
            ".scope { --local: red; } .scope.active > span { color: var(--local); }",
            ".scope { --local: red; } section.scope span { color: var(--local); }",
            "#scope { --local: red; } main #scope > span { color: var(--local); }",
            ".scope { --local: red; } .scope span + em { color: var(--local); }",
            ".scope { --local: red; } .scope > span ~ em { color: var(--local); }",
            ".scope { --local: red; } .scope span, .scope > em { color: var(--local); }",
            ".one, .two { --local: red; } .one span, .two em { color: var(--local); }",
            "@layer defaults { .scope { --local: red; } } .scope span { color: var(--local); }",
            ".scope { --local: red; } @media (width > 1px) { .scope span { color: var(--local); } }",
            ".scope /* default */ { --local: red; } .scope /* child */ > span { color: var(--local); }",
            ".caf\u{e9} { --local: red; } .caf\u{e9} span { color: var(--local); }",
            ".\\73 cope { --local: red; } .\\73 cope span { color: var(--local); }",
        ] {
            assert!(CssParser::new().extract_tokens(css).unwrap().is_empty(), "{css}");
        }
    }

    #[test]
    fn scoped_defaults_do_not_hide_unproven_requirements() {
        for css in [
            ".scope { --local: red; } span { color: var(--local); }",
            ".scope { --local: red; } .scope span { color: var(--local); } .other { color: var(--local); }",
            ".scope { --local: red; } .scope-other span { color: var(--local); }",
            ".scope { --local: red; } .scope + span { color: var(--local); }",
            ".scope { --local: red; } .scope ~ span em { color: var(--local); }",
            ".scope { --local: red; } .scope span, .other em { color: var(--local); }",
            ".scope { --local: red; } .scope span, .scope + em { color: var(--local); }",
            ".scope:hover { --local: red; } .scope span { color: var(--local); }",
            ":is(.one, .scope, .two) { --local: red; } .scope span { color: var(--local); }",
            ".other\\,scope { --local: red; } scope span { color: var(--local); }",
            ".outer .scope { --local: red; } .scope span { color: var(--local); }",
            "@media (width > 1px) { .scope { --local: red; } } .scope span { color: var(--local); }",
            "@supports (display: grid) { .scope { --local: red; } } .scope span { color: var(--local); }",
            "@container (width > 1px) { .scope { --local: red; } } .scope span { color: var(--local); }",
            "@media print { @layer defaults { .scope { --local: red; } } } .scope span { color: var(--local); }",
            ".outer { .scope { --local: red; } } .scope span { color: var(--local); }",
            ".scope { --local: red; } :not(.scope) span { color: var(--local); }",
            ".scope { --local: red; } :is(.scope, .other) span { color: var(--local); }",
            ".scope { --local: red; } .scope/**/.active + span { color: var(--local); }",
            ".scope { --local: red; } .scope::slotted(span) { color: var(--local); }",
            ".scope { --local: red; } .scope span:hover { color: var(--local); }",
            ".scope { --local: red; } .scope span::before { color: var(--local); }",
            ".scope { --local: red; } .scope [title] { color: var(--local); }",
            ".scope { --local: red; } .scope * { color: var(--local); }",
            ".scope { --local: red; } .scope { span { color: var(--local); } }",
        ] {
            assert_eq!(
                CssParser::new().extract_tokens(css).unwrap(),
                HashSet::from(["local".to_string()]),
                "{css}",
            );
        }
    }

    #[test]
    fn pruning_preserves_alias_dependencies_and_optional_fallbacks() {
        let css = ".scope { --local: var(--brand); }
            .scope span { color: var(--local, var(--optional, red)); }
            .other { color: var(--local); }";
        let (definitions, requirements, _, _) = CssParser::new()
            .extract_definitions_requirements_and_strip_comments(css, LegalComments::Inline)
            .unwrap();
        assert!(
            definitions.is_empty(),
            "scoped defaults must not escape the stylesheet"
        );
        assert_eq!(requirements.len(), 3);
        assert_eq!(requirements[0].tokens, ["brand"]);
        assert!(!requirements[0].has_literal_fallback);
        assert_eq!(requirements[1].tokens, ["optional"]);
        assert!(requirements[1].has_literal_fallback);
        assert_eq!(requirements[2].tokens, ["local"]);
        assert!(!requirements[2].has_literal_fallback);
    }

    #[test]
    fn scoped_defaults_do_not_leak_between_scans() {
        let mut parser = CssParser::new();
        parser.extract_tokens(".scope { --local: red; }").unwrap();
        assert_eq!(
            parser
                .extract_tokens(".scope span { color: var(--local); }")
                .unwrap(),
            HashSet::from(["local".to_string()]),
        );
    }
}
