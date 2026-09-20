// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::HashMap;

use crate::{codes, Diagnostic, Fragment, HtmlParser};

impl HtmlParser {
    pub(super) fn finalize_outlet_warnings(&mut self) {
        let counts = self.route_level_outlet_counts();
        // Owner indices name authored templates, not synthetic directive records.
        let mut owners: Vec<&str> = self
            .loop_owner_indices
            .keys()
            .map(String::as_str)
            .filter(|owner| counts.get(owner).is_some_and(|count| *count > 1))
            .collect();
        owners.sort_unstable();
        for owner in owners {
            // Keep the more precise source-positioned warning when parsing
            // already found multiple outlets within this authored template.
            if !self.warnings.iter().any(|warning| {
                warning.error_code() == Some(codes::MULTIPLE_OUTLETS)
                    && warning.component_name() == Some(owner)
            }) {
                self.warnings.push(multiple_outlets_warning(owner));
            }
        }
    }

    fn route_level_outlet_counts(&self) -> HashMap<&str, u8> {
        let hosts = self.outlet_host_records();
        if hosts.is_empty() {
            return HashMap::new();
        }

        let mut parents: HashMap<&str, Vec<&str>> =
            HashMap::with_capacity(self.fragment_records.len());
        let mut pending = Vec::with_capacity(hosts.len());
        for (id, list) in &self.fragment_records {
            if hosts.contains(id.as_str()) {
                let count = list
                    .fragments
                    .iter()
                    .filter(|fragment| {
                        matches!(fragment.fragment.as_ref(), Some(Fragment::Outlet(_)))
                    })
                    .take(2)
                    .count();
                pending.push((id.as_str(), if count == 1 { 1 } else { 2 }));
            }
            for fragment in &list.fragments {
                let target = match fragment.fragment.as_ref() {
                    Some(Fragment::Component(component)) => &component.fragment_id,
                    Some(Fragment::IfCond(condition)) => &condition.fragment_id,
                    Some(Fragment::ForLoop(repeat)) => &repeat.fragment_id,
                    // A route installs its own route_children. Neither route
                    // edges nor outlet-mounted child routes share this level.
                    _ => continue,
                };
                // Preserve repeated callsites, even when they target the same
                // record: two instances can each render an outlet.
                parents.entry(target.as_str()).or_default().push(id);
            }
        }

        let mut counts: HashMap<&str, u8> = HashMap::with_capacity(self.fragment_records.len());
        // Propagate only increases, capped at two. Each record changes at most
        // twice, so repeated edges and named-loop cycles remain linear.
        while let Some((id, added)) = pending.pop() {
            let count = counts.entry(id).or_default();
            let next = (*count + added).min(2);
            let delta = next - *count;
            if delta == 0 {
                continue;
            }
            *count = next;
            if let Some(incoming) = parents.get(id) {
                pending.extend(incoming.iter().map(|parent| (*parent, delta)));
            }
        }
        counts
    }
}

#[cold]
#[inline(never)]
pub(super) fn multiple_outlets_warning(owner: &str) -> Diagnostic {
    Diagnostic::warning("multiple <outlet> elements at one route level")
        .code(codes::MULTIPLE_OUTLETS)
        .component(owner)
        .element("outlet")
        .help(
            "only the first <outlet> at a route level currently renders matched child routes; remove the extra <outlet>, including outlets in nested components, or move duplicated layout into the matched route component",
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ComponentRegistration, DomStrategy};

    fn parse_shell(shell: &str) -> HtmlParser {
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        for (tag, template) in [
            ("app-shell", shell),
            ("nested-layout", "<outlet />"),
            ("wrapper-layout", "<nested-layout></nested-layout>"),
            ("child-page", "<outlet />"),
        ] {
            parser
                .component_registry_mut()
                .register_component(ComponentRegistration::new(tag, template, None, false))
                .unwrap();
        }
        parser
            .parse(
                "index.html",
                r#"<route path="/" component="app-shell"><route path="child" component="child-page" /></route>"#,
            )
            .unwrap();
        parser
    }

    #[test]
    fn warns_for_cross_component_outlet_callsites() {
        for shell in [
            "<nested-layout></nested-layout><outlet />",
            "<outlet /><nested-layout></nested-layout>",
            "<nested-layout></nested-layout><nested-layout></nested-layout>",
            "<wrapper-layout></wrapper-layout><nested-layout></nested-layout>",
            r#"<if condition="show"><wrapper-layout></wrapper-layout></if><outlet />"#,
            r#"<for each="item in items"><nested-layout></nested-layout></for><outlet />"#,
            r#"<for id="reuse" each="item in items"><nested-layout></nested-layout></for><for id="reuse" each="item in others" />"#,
        ] {
            let mut parser = parse_shell(shell);
            let warnings = parser.take_warnings();
            assert_eq!(warnings.len(), 1, "{shell}: {warnings:?}");
            assert_eq!(warnings[0].error_code(), Some(codes::MULTIPLE_OUTLETS));
            assert_eq!(warnings[0].component_name(), Some("app-shell"));
            assert!(warnings[0].help_text().is_some());
            assert!(parser.take_warnings().is_empty());
        }
    }

    #[test]
    fn keeps_route_levels_and_unreferenced_templates_separate() {
        for shell in [
            "<outlet />",
            "<wrapper-layout></wrapper-layout>",
            r#"<if condition="show"><nested-layout></nested-layout></if>"#,
            r#"<outlet /><route path="/other" component="nested-layout" />"#,
        ] {
            let mut parser = parse_shell(shell);
            assert!(parser.take_warnings().is_empty(), "{shell}");
        }
    }

    #[test]
    fn preserves_positioned_local_warning_without_duplicating_it() {
        let mut parser = parse_shell("<outlet />\n<outlet />");
        let warnings = parser.take_warnings();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].component_name(), Some("app-shell"));
        assert_eq!(warnings[0].position_line_column(), Some((2, 1)));
    }

    #[test]
    fn named_loop_cycles_converge_and_count_reachable_outlets() {
        for (body, expected) in [("<outlet />", true), ("<span>item</span>", false)] {
            let mut parser = parse_shell(&format!(
                r#"<for id="tree" each="item in items">{body}<for id="tree" each="item in item.children" /></for>"#
            ));
            assert_eq!(!parser.take_warnings().is_empty(), expected);
        }
    }
}
