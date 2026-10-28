// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fmt::Write;

/// The static topology needed to adopt SSR DOM without parsing client HTML.
#[derive(Default)]
pub(super) struct SsrLayout {
    /// Direct element count * 2, plus whether this section may own child nodes.
    elements: Vec<usize>,
    /// Node-kind ordinal of the next static sibling for each text slot; 0 is end.
    texts: Vec<usize>,
}

impl SsrLayout {
    pub(super) fn new() -> Self {
        Self {
            elements: vec![1],
            ..Self::default()
        }
    }

    pub(super) fn push_element(&mut self, parent: usize, tag: &str) {
        if let Some(shape) = self.elements.get_mut(parent) {
            *shape += 2;
            self.elements.push(usize::from(!tag.contains('-')));
        }
    }

    pub(super) fn finish_children(
        &mut self,
        parent: usize,
        children: &SsrChildren,
        text_count: usize,
    ) {
        let Some(shape) = self.elements.get_mut(parent) else {
            return; // Inert <template> content is not in the numbered DOM.
        };
        *shape |= usize::from(children.count != 0);
        self.texts.resize(text_count, 0);
    }

    pub(super) fn emit(&self, out: &mut String) {
        // A layout format change also renames this key, so a compiler and
        // runtime from mismatched releases reject each other's metadata.
        out.push_str(",\"l\":[");
        emit_numbers(&self.elements, out);
        out.push(']');
    }

    pub(super) fn text_successor(&self, index: usize) -> usize {
        self.texts[index]
    }
}

#[derive(Default)]
pub(super) struct SsrChildren {
    count: usize,
    ordinals: [usize; 4],
    pending: Option<usize>,
}

impl SsrChildren {
    pub(super) fn text(&mut self, layout: &mut SsrLayout, index: usize) {
        if layout.elements.is_empty() {
            return;
        }
        layout.texts.resize(index + 1, 0);
        self.pending.get_or_insert(index);
    }

    // Resolve local text runs before descending into the next static element.
    // Their indices are still contiguous, so no per-parent vectors or scans of
    // descendant bindings are needed.
    pub(super) fn node(&mut self, layout: &mut SsrLayout, kind: u8) {
        let code = kind_code(kind);
        if let Some(start) = self.pending.take() {
            layout.texts[start..].fill(self.ordinals[code] * 4 + code);
        }
        self.ordinals[code] += 1;
        self.count += 1;
    }
}

fn kind_code(kind: u8) -> usize {
    match kind {
        1 => 1,
        3 => 2,
        _ => 3,
    }
}

fn emit_numbers(values: &[usize], out: &mut String) {
    for (i, value) in values.iter().enumerate() {
        if i != 0 {
            out.push(',');
        }
        let _ = write!(out, "{value}");
    }
}

#[cfg(test)]
mod tests {
    use super::super::compile_to_metadata;

    #[test]
    fn topology_preserves_custom_element_ownership_and_inert_templates() -> crate::Result<()> {
        let meta = compile_to_metadata(
            "layout-test",
            "<section><my-child></my-child><my-child><span>slot</span></my-child><template><b>inert</b></template><input></section>".into(),
            Vec::new(),
        )?;
        assert_eq!(meta.root.ssr_layout.elements, [3, 9, 0, 3, 1, 1, 1]);
        Ok(())
    }

    #[test]
    fn text_successors_keep_node_kind_ordinals() -> crate::Result<()> {
        let meta = compile_to_metadata(
            "layout-test",
            "<div>{{a}}<span>x</span>{{b}}<!-- @license MIT -->{{c}}<i></i>{{d}}</div>".into(),
            Vec::new(),
        )?;
        // Authored HTML comments are stripped, merging the adjacent b/c run.
        assert_eq!(meta.root.ssr_layout.texts, [1, 5, 0]);
        assert_eq!(meta.root.ssr_layout.elements, [3, 5, 1, 1]);
        Ok(())
    }

    #[test]
    fn serialized_layout_omits_zero_successors_without_changing_raw_flags() -> crate::Result<()> {
        let output = super::super::generate_compiled_template(
            "layout-test",
            "<div>{{a}}<span>x</span>{{{b}}}<i></i>{{c}}</div>",
        )?;
        assert!(output.contains(r#""l":[3,5,1,1]"#), "{output}");
        assert!(!output.contains(r#""s":"#), "{output}");
        assert!(
            output.contains(r#""tx":[[[1,0],[["a"]],0,1],[[1,1],[["b"]],1,5],[[1,2],[["c"]]]]"#),
            "{output}"
        );
        Ok(())
    }

    #[test]
    fn static_successors_distinguish_text_comments_and_elements() {
        let mut layout = super::SsrLayout::new();
        let mut children = super::SsrChildren::default();
        for (index, kind) in [1, 8, 3, 1].into_iter().enumerate() {
            children.text(&mut layout, index);
            children.node(&mut layout, kind);
        }
        children.text(&mut layout, 4);
        layout.finish_children(0, &children, 5);
        assert_eq!(layout.texts, [1, 3, 2, 5, 0]);
    }

    #[test]
    fn implied_table_containers_are_in_the_ssr_topology() -> crate::Result<()> {
        let meta = compile_to_metadata(
            "layout-test",
            "<table><tr><td>{{cell}}</td></tr></table>".into(),
            Vec::new(),
        )?;
        assert_eq!(meta.root.ssr_layout.elements, [3, 3, 3, 3, 1]);
        assert_eq!(meta.root.ssr_layout.texts, [0]);
        Ok(())
    }

    #[test]
    fn structural_bodies_have_independent_layouts() -> crate::Result<()> {
        let meta = compile_to_metadata(
            "layout-test",
            r#"<main><if condition="show"><p>{{label}}</p></if><for each="item in items"><span>{{item}}</span></for></main>"#.into(),
            Vec::new(),
        )?;
        assert_eq!(meta.root.ssr_layout.elements, [3, 1]);
        assert!(meta.root.ssr_layout.texts.is_empty());
        for block in &meta.blocks {
            assert_eq!(block.ssr_layout.elements, [3, 1]);
            assert_eq!(block.ssr_layout.texts, [0]);
        }
        Ok(())
    }
}
