// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{CompiledRepeat, ParsedForBlock, RootScope, TemplateSectionMeta};
use crate::{NamedForLoops, Result};

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
        repeat: &ParsedForBlock,
    ) -> Result<usize> {
        let Some(id) = &repeat.id else {
            return Ok(self.reserve());
        };
        self.named_repeat_index(component, repeat, id)
    }

    fn named_repeat_index(
        &mut self,
        component: &str,
        repeat: &ParsedForBlock,
        id: &str,
    ) -> Result<usize> {
        self.validation
            .register(component, id, &repeat.item_var, (repeat.definition, 0))?;
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
            // Do not deduplicate across other callsites: their scopes may differ.
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
        generate_compiled_template,
    };
    use crate::diagnostic::codes;
    use crate::ParserError;

    #[test]
    fn self_reference_reuses_one_body_and_normalizes_condition_braces() {
        let source = r#"<for id="tree-item" each="child in items"><li>{{child.name}}</li><if condition="{{child.children}}"><ul><for id="tree-item" each="child in child.children" /></ul></if></for>"#;
        let meta = compile_to_metadata("test-tree", source, Vec::new()).unwrap();
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
        let meta = compile_to_metadata("test-tree", source, Vec::new()).unwrap();
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
        let meta = compile_to_metadata("test-tree", source, Vec::new()).unwrap();
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
        let meta = compile_to_metadata("test-tree", source, Vec::new()).unwrap();
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
            let meta = compile_to_metadata(tag, r#"<for id="node" each="item in items"><i>{{item.name}}</i><for id="node" each="item in item.children"/></for>"#, Vec::new()).unwrap();
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
            let result = compile_to_metadata("test-tree", source, Vec::new());
            let Err(ParserError::Template(diagnostic)) = result else {
                panic!("expected {code}: {source}");
            };
            assert_eq!(diagnostic.error_code(), Some(code));
            assert!(diagnostic.help_text().is_some());
        }
    }
}
