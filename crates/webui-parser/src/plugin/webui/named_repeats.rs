// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{CompiledRepeat, ParsedForBlock, RootScope, TemplateSectionMeta};
use crate::scoped_visits::ScopedVisits;
use crate::{NamedForLoops, Result};

pub(super) fn visit_scope<'a>(
    visits: &mut ScopedVisits<'a, *const TemplateSectionMeta>,
    block: &TemplateSectionMeta,
    scopes: &[RootScope<'a>],
    scope: Option<usize>,
) -> bool {
    let mut current = scope;
    visits.insert(
        // Metadata stays immutably borrowed throughout analysis, so its address
        // identifies the block without enlarging ordinary traversal frames.
        std::ptr::from_ref(block),
        std::iter::from_fn(|| {
            let frame = &scopes[current?];
            current = frame.parent;
            Some(frame.name)
        }),
    )
}

#[derive(Default)]
pub(super) struct CompileBlocks {
    pub(super) blocks: Vec<TemplateSectionMeta>,
    named: Vec<NamedBlock>,
    validation: NamedForLoops,
}

struct NamedBlock {
    id: String,
    block_index: usize,
    key_path: Option<String>,
}

impl CompileBlocks {
    pub(super) fn has_named_repeats(&self) -> bool {
        !self.named.is_empty()
    }

    #[inline]
    pub(super) fn reserve(&mut self) -> usize {
        let index = self.blocks.len();
        self.blocks.push(TemplateSectionMeta::default());
        index
    }

    #[inline]
    pub(super) fn repeat_index(
        &mut self,
        component: &str,
        repeat: &ParsedForBlock<'_>,
    ) -> Result<usize> {
        let Some(id) = &repeat.id else {
            return Ok(self.reserve());
        };
        self.named_repeat_index(component, repeat, id)
    }

    fn named_repeat_index(
        &mut self,
        component: &str,
        repeat: &ParsedForBlock<'_>,
        id: &str,
    ) -> Result<usize> {
        self.validation
            .register(
                component,
                id,
                &repeat.item_var,
                (repeat.definition, repeat.offset),
            )
            .map_err(|error| {
                let source = super::TemplateSource {
                    source: repeat.body.source,
                    range: repeat.offset..repeat.offset,
                    repeat_key: None,
                };
                source.locate_error(error)
            })?;
        if let Some(named) = self.named.iter_mut().find(|named| named.id == id) {
            if repeat.definition {
                named.key_path.clone_from(&repeat.key_path);
            }
            return Ok(named.block_index);
        }
        let block_index = self.reserve();
        self.named.push(NamedBlock {
            id: id.to_owned(),
            block_index,
            key_path: repeat.key_path.clone(),
        });
        Ok(block_index)
    }

    pub(super) fn resolve(
        &mut self,
        component: &str,
        source: &str,
        root: &mut TemplateSectionMeta,
    ) -> Result<()> {
        if self.named.is_empty() {
            return Ok(());
        }
        self.validation.validate(component, source)?;
        inherit_keys(&mut root.repeats, &self.named);
        for block in &mut self.blocks {
            inherit_keys(&mut block.repeats, &self.named);
        }
        Ok(())
    }
}

fn inherit_keys(repeats: &mut [CompiledRepeat], named: &[NamedBlock]) {
    for repeat in repeats {
        if let Some(named) = named
            .iter()
            .find(|named| named.block_index == repeat.block_index)
        {
            repeat.key_path.clone_from(&named.key_path);
        }
    }
}

pub(super) fn is_ancestor_block(
    block_index: usize,
    scopes: &[RootScope<'_>],
    scope: Option<usize>,
) -> bool {
    let mut current = scope;
    while let Some(index) = current {
        let Some(frame) = scopes.get(index) else {
            return false;
        };
        if frame.block_index == block_index {
            // A cycle adds only item scopes, so it cannot reveal a new free root.
            // Other callsites are deduplicated separately by their scope sets.
            return true;
        }
        current = frame.parent;
    }
    false
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]

    use super::super::{
        collect_template_build_metadata, compile_to_metadata, emit_js_condition_function,
        generate_compiled_template, RootScope,
    };
    use super::{visit_scope, ScopedVisits};
    use crate::diagnostic::codes;
    use crate::ParserError;

    #[test]
    fn equivalent_scope_sets_ignore_order_shadowing_and_callsite_identity() {
        let first = super::TemplateSectionMeta::default();
        let second = super::TemplateSectionMeta::default();
        let scopes = [
            RootScope {
                name: "a",
                block_index: 0,
                parent: None,
            },
            RootScope {
                name: "b",
                block_index: 1,
                parent: Some(0),
            },
            RootScope {
                name: "b",
                block_index: 2,
                parent: None,
            },
            RootScope {
                name: "a",
                block_index: 3,
                parent: Some(2),
            },
            RootScope {
                name: "a",
                block_index: 4,
                parent: Some(3),
            },
        ];
        let mut visits = ScopedVisits::default();
        assert!(visit_scope(&mut visits, &first, &scopes, Some(1)));
        assert!(!visit_scope(&mut visits, &first, &scopes, Some(3)));
        assert!(!visit_scope(&mut visits, &first, &scopes, Some(4)));
        assert!(visit_scope(&mut visits, &first, &scopes, Some(0)));
        assert!(visit_scope(&mut visits, &second, &scopes, Some(1)));
        assert!(visit_scope(&mut visits, &first, &scopes, None));
    }

    #[test]
    fn shared_diamond_paths_retain_only_linear_scope_frames() {
        use std::fmt::Write;

        const COUNT: usize = 12;
        let mut source = String::with_capacity(COUNT * 150);
        for index in 0..COUNT {
            write!(source, r#"<for id="b{index}" each="item in items">"#).unwrap();
            if index + 1 < COUNT {
                let next = index + 1;
                write!(source, r#"<for id="b{next}" each="item in item.children"/><for id="b{next}" each="item in item.children"/>"#).unwrap();
            } else {
                source.push_str("<b>{{item.name}} {{title}}</b>");
            }
            source.push_str("</for>");
        }
        let meta = compile_to_metadata("test-tree", source.as_str().into(), Vec::new()).unwrap();
        assert_eq!(meta.blocks.len(), COUNT);
        let metadata = collect_template_build_metadata(&meta);
        assert_eq!(metadata.roots, ["items", "title"]);
        assert_eq!(metadata.scope_count, COUNT + 2 * (COUNT - 1));
    }

    #[test]
    fn equivalent_visits_preserve_first_processed_root_order() {
        let source = r#"<for id="a" each="item in first"><if condition="item.visible"><b>{{alpha}}</b></if></for><for each="item in middle"><b>{{middleRoot}}</b></for><for id="a" each="item in last"/>"#;
        let meta = compile_to_metadata("test-tree", source.into(), Vec::new()).unwrap();
        assert_eq!(
            collect_template_build_metadata(&meta).roots,
            ["first", "middle", "last", "alpha", "middleRoot"]
        );
    }

    #[test]
    fn shared_body_keeps_roots_visible_in_different_scope_sets() {
        // Both scopes have the same size and bind the same immediate item.
        // Neither scope is a subset of the other.
        let source = r#"<for each="left in lefts"><for id="shared" each="item in left.items"><if condition="item.visible"><p title="{{left.label}}">{{right.label}} {{item.name}}</p></if></for></for><for each="right in rights"><for id="shared" each="item in right.items"/></for>"#;
        let meta = compile_to_metadata("test-tree", source.into(), Vec::new()).unwrap();
        let mut roots = collect_template_build_metadata(&meta).roots;
        roots.sort();
        assert_eq!(roots, ["left", "lefts", "right", "rights"]);
    }

    #[test]
    fn self_reference_reuses_one_body_and_normalizes_condition_braces() {
        let source = r#"<for id="tree-item" each="child in items"><li>{{child.name}}</li><if condition="{{child.children}}"><ul><for id="tree-item" each="child in child.children" /></ul></if></for>"#;
        let meta = compile_to_metadata("test-tree", source.into(), Vec::new()).unwrap();
        assert_eq!(meta.blocks.len(), 2);
        assert_eq!(meta.root.repeats[0].block_index, 0);
        assert_eq!(meta.root.repeats[0].collection, "items");
        assert_eq!(meta.root.repeats[0].item_var, "child");
        assert_eq!(meta.blocks[1].repeats[0].block_index, 0);
        assert_eq!(meta.blocks[1].repeats[0].collection, "child.children");
        let metadata = collect_template_build_metadata(&meta);
        assert_eq!(metadata.roots, ["items"]);
        let mut condition = String::new();
        emit_js_condition_function(&meta.blocks[0].conditionals[0].0, &mut condition);
        assert!(condition.contains(r#"v("child.children",s)"#));
        assert!(!condition.contains(r#"{{child.children}}"#));
    }

    #[test]
    fn forward_references_inherit_definition_keys_without_copying_blocks() {
        let source = r#"<for id="node" each="item in first"/><for id="node" each="item in second"><span key="{{item.id}}">{{item.name}}</span><for id="node" each="item in item.children"/></for>"#;
        let meta = compile_to_metadata("test-tree", source.into(), Vec::new()).unwrap();
        assert_eq!(meta.blocks.len(), 1);
        assert_eq!(meta.root.repeats.len(), 2);
        for repeat in meta.root.repeats.iter().chain(&meta.blocks[0].repeats) {
            assert_eq!(repeat.block_index, 0);
            assert_eq!(repeat.key_path.as_deref(), Some("id"));
        }
        assert!(!meta.blocks[0].html.contains("key="));
        assert_eq!(
            collect_template_build_metadata(&meta).roots,
            ["first", "second"]
        );
    }

    #[test]
    fn mutual_references_are_finite_and_collect_roots_in_every_callsite_scope() {
        let source = r#"<for each="outside in groups"><for id="a" each="node in outside.nodes"><span>{{outside.label}} {{title}} {{node.name}}</span><for id="b" each="node in node.children"/></for></for><for id="a" each="node in roots"/><for id="b" each="node in extras"><button @click="{select(node.id)}">{{footer}}</button><for id="a" each="node in node.children"/></for>"#;
        let meta = compile_to_metadata("test-tree", source.into(), Vec::new()).unwrap();
        assert_eq!(meta.blocks.len(), 3);
        assert_eq!(meta.blocks[1].repeats[0].block_index, 2);
        assert_eq!(meta.blocks[2].repeats[0].block_index, 1);
        let metadata = collect_template_build_metadata(&meta);
        let mut roots = metadata.roots;
        roots.sort();
        assert_eq!(
            roots,
            ["extras", "footer", "groups", "outside", "roots", "title"]
        );
        assert!(metadata.has_events);
    }

    #[test]
    fn item_scopes_are_restored_for_siblings_after_recursive_blocks() {
        let source = r#"<for id="node" each="item in items"><span>{{item.name}}</span><for id="node" each="item in item.children"/></for><p>{{item.name}}</p>"#;
        let meta = compile_to_metadata("test-tree", source.into(), Vec::new()).unwrap();
        let mut roots = collect_template_build_metadata(&meta).roots;
        roots.sort();
        assert_eq!(roots, ["item", "items"]);
    }

    #[test]
    fn names_are_component_local_and_ordinary_metadata_is_unchanged() {
        let ordinary =
            r#"<for each="item in items"><span key="{{item.id}}">{{item.name}}</span></for>"#;
        let named = ordinary.replace("<for each", "<for id=\"node\" each");
        let ordinary = generate_compiled_template("test-tree", ordinary).unwrap();
        let named = generate_compiled_template("test-tree", &named).unwrap();
        assert_eq!(ordinary, named);
        for tag in ["test-first", "test-second"] {
            let meta = compile_to_metadata(tag, r#"<for id="node" each="item in items"><i>{{item.name}}</i><for id="node" each="item in item.children"/></for>"#.into(), Vec::new()).unwrap();
            assert_eq!(meta.blocks.len(), 1);
            assert_eq!(meta.blocks[0].repeats[0].block_index, 0);
        }
    }

    #[test]
    fn braced_and_unbraced_conditions_emit_identical_metadata() {
        let ordinary = r#"<for each="item in items"><if condition="item.children"><span>{{item.name}}</span></if></for>"#;
        let braced = ordinary.replace(
            "condition=\"item.children\"",
            "condition=\"{{item.children}}\"",
        );
        assert_eq!(
            generate_compiled_template("test-tree", ordinary).unwrap(),
            generate_compiled_template("test-tree", &braced).unwrap()
        );
    }

    #[test]
    fn invalid_named_loops_return_shared_actionable_diagnostics() {
        for (source, code) in [
            (
                r#"<for template="node" each="item in items"><span>{{item.name}}</span></for>"#,
                codes::INVALID_FOR_ID,
            ),
            (
                r#"<for template="node" each="item in items"/>"#,
                codes::INVALID_FOR_ID,
            ),
            (
                r#"<for template each="item in items"></for>"#,
                codes::INVALID_FOR_ID,
            ),
            (
                r#"<for id="" each="item in items"/>"#,
                codes::INVALID_FOR_ID,
            ),
            (
                r#"<for id="{{name}}" each="item in items"/>"#,
                codes::INVALID_FOR_ID,
            ),
            (
                r#"<for id="node" template="node" each="item in items"/>"#,
                codes::INVALID_FOR_ID,
            ),
            (
                r#"<for id="node" each="item in items"/>"#,
                codes::UNKNOWN_FOR_ID,
            ),
            (
                r#"<for id="node" each="item in items"><i/></for><for id="node" each="item in items"><b/></for>"#,
                codes::DUPLICATE_FOR_ID,
            ),
            (
                r#"<for id="node" each="item in items"><for id="node" each="child in item.children"><b>{{child.name}}</b></for></for>"#,
                codes::DUPLICATE_FOR_ID,
            ),
            (
                r#"<for id="node" each="item in items"><i/></for><for id="node" each="child in children"/>"#,
                codes::INCOMPATIBLE_FOR_ITEM,
            ),
        ] {
            let result = compile_to_metadata("test-tree", source.into(), Vec::new());
            let Err(ParserError::Template(diagnostic)) = result else {
                panic!("expected {code}: {source}");
            };
            assert_eq!(diagnostic.error_code(), Some(code));
            assert!(diagnostic.help_text().is_some());
        }
    }

    fn assert_diagnostic_site(source: &str, code: &'static str, position: (usize, usize)) {
        let server = crate::HtmlParser::new()
            .parse("test-tree", source)
            .unwrap_err();
        let client = generate_compiled_template("test-tree", source).unwrap_err();
        for error in [server, client] {
            let ParserError::Template(diagnostic) = error else {
                panic!("expected an authoring diagnostic");
            };
            assert_eq!(diagnostic.error_code(), Some(code));
            assert_eq!(diagnostic.position_line_column(), Some(position));
            assert!(diagnostic.help_text().is_some());
        }
    }

    #[test]
    fn unresolved_named_repeats_report_the_first_source_site_deterministically() {
        let source = concat!(
            "\r\n",
            "<!-- header -->\r\n",
            "<template shadowrootmode=\"open\">\r\n",
            "  <for each=\"row in rows\">\r\n",
            "    <if condition=\"row.visible\">\r\n",
            "      <section key=\"{{row.id}}\">\r\n",
            "        <for id=\"z-first\" each=\"item in row.children\" />\r\n",
            "      </section>\r\n",
            "    </if>\r\n",
            "  </for>\r\n",
            "  <for id=\"a-later\" each=\"item in items\" />\r\n",
            "</template>\r\n",
        );
        for _ in 0..32 {
            assert_diagnostic_site(source, codes::UNKNOWN_FOR_ID, (7, 9));
        }
    }

    #[test]
    fn named_repeat_errors_preserve_sites_inside_keyed_conditional_bodies() {
        for (reference, code) in [
            (
                r#"<for id="node" each="item in item.children"><b>duplicate</b></for>"#,
                codes::DUPLICATE_FOR_ID,
            ),
            (
                r#"<for id="node" each="child in item.children" />"#,
                codes::INCOMPATIBLE_FOR_ITEM,
            ),
            (
                r#"<for id="" each="item in item.children" />"#,
                codes::INVALID_FOR_ID,
            ),
        ] {
            let source = format!(
                "\n<template shadowrootmode=\"open\">\n  <for id=\"node\" each=\"item in items\">\n    <if condition=\"item.visible\">\n      <section key=\"{{{{item.id}}}}\">\n        {reference}\n      </section>\n    </if>\n  </for>\n</template>"
            );
            assert_diagnostic_site(&source, code, (6, 9));
        }
    }

    #[test]
    fn named_repeat_error_columns_are_not_shifted_by_unicode_or_stripped_keys() {
        let source = concat!(
            "\n<for id=\"node\" each=\"item in items\"><p key=\"{{item.id}}\">",
            "\u{00e9}</p><for id=\"node\" each=\"child in item.children\" /></for>"
        );
        assert_diagnostic_site(source, codes::INCOMPATIBLE_FOR_ITEM, (2, 63));
    }
}
