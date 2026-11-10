// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::skip_css_trivia;
use crate::css_scan;
use std::collections::HashSet;
use webui_tokens::CssFallbackChain;

#[derive(Default)]
pub(super) struct CssInheritance<'a> {
    definitions: HashSet<(&'a str, &'a str)>,
}

impl<'a> CssInheritance<'a> {
    pub(super) fn define(&mut self, source: &'a str, scope_id: usize, name: &'a str) {
        let Some(selector) = scope_id
            .checked_sub(1)
            .and_then(|start| source.get(start..))
        else {
            return;
        };
        if !simple_selector_list(selector, |_| {}) {
            return;
        }
        simple_selector_list(selector, |branch| {
            self.definitions.insert((branch, name));
        });
    }

    pub(super) fn resolve(
        &self,
        source: &str,
        local_definitions: &HashSet<&str>,
        requirements: &mut Vec<CssFallbackChain>,
        scopes: &[usize],
    ) {
        if self.definitions.is_empty() {
            return;
        }
        for (requirement, scope_id) in requirements.iter_mut().zip(scopes) {
            let Some(selector) = scope_id
                .checked_sub(1)
                .and_then(|start| source.get(start..))
            else {
                continue;
            };
            requirement.tokens.retain(|name| {
                !local_definitions.contains(name.as_str()) || !self.covers(selector, name.as_str())
            });
        }
        requirements.retain(|requirement| !requirement.tokens.is_empty());
    }

    fn covers(&self, mut selector: &str, name: &str) -> bool {
        let mut inherited = false;
        selector = skip_css_trivia(selector);
        loop {
            let Some((rest, subject)) = self.compound(selector, name) else {
                return false;
            };
            let tail = skip_css_trivia(rest);
            match tail.as_bytes().first() {
                Some(b'{') => return inherited || subject,
                Some(b',') if inherited || subject => inherited = false,
                Some(b'>') => inherited |= subject,
                // Siblings retain an earlier ancestor, but not the previous subject.
                Some(b'+' | b'~') => {}
                _ if rest.starts_with(char::is_whitespace) => {
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
        let mut covered = self.definitions.contains(&(&selector[..end], name));
        while matches!(selector.as_bytes().get(end), Some(b'.' | b'#')) {
            let next = simple_selector_end(&selector[end..]);
            if next == 0 {
                return None;
            }
            covered |= self
                .definitions
                .contains(&(&selector[end..end + next], name));
            end += next;
        }
        Some((&selector[end..], covered))
    }
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
    use crate::{CssParser, LegalComments};
    use std::collections::HashSet;

    #[test]
    fn simple_defaults_cover_proven_subjects_and_ancestors() {
        for css in [
            "local-token { --local: red; } local-token span { color: var(--local); }",
            "local-token span { color: var(--local); } local-token { --local: red; }",
            ".scope { --local: red; } .scope { color: var(--local); }",
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
        let (definitions, requirements, _) = CssParser::new()
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
