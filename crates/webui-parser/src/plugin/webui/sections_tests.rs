// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use super::*;

#[test]
fn suspended_stack_starts_at_one_slot_and_reuses_it() {
    let mut suspended = Vec::new();
    assert_eq!(suspended.capacity(), 0);
    push_suspended_cursor(&mut suspended, SectionFrame::new(0..0, None, None).cursor);
    assert_eq!(suspended.capacity(), 1);
    assert!(suspended.pop().is_some());
    push_suspended_cursor(&mut suspended, SectionFrame::new(0..0, None, None).cursor);
    assert_eq!(suspended.capacity(), 1);
    assert_eq!(suspended.len(), 1);
}

#[test]
fn local_active_frame_handles_flat_templates() -> Result<()> {
    let meta = compile_to_metadata("flat-view", "<span>value</span>", Vec::new())?;
    assert_eq!(meta.root.html, "<span>value</span>");
    assert!(meta.blocks.is_empty());
    Ok(())
}

#[test]
fn markup_dispatch_preserves_literal_scalar_advancement() -> Result<()> {
    for (source, first) in [
        ("text<if condition=\"ready\">body</if>", "t"),
        (" \n<render fragment=\"later\"/>", " "),
        ("\u{1f642}{{name}}", "\u{1f642}"),
        ("@example.test", "@"),
        ("{literal}", "{"),
    ] {
        let mut compiler = SectionCompiler {
            component: "literal-view",
            source,
            declarations: None,
            blocks: Vec::new(),
        };
        let mut frame = SectionFrame::new(0..source.len(), None, None);
        assert!(compiler.advance(&mut frame)?.is_none());
        assert_eq!(frame.cursor.position, first.len(), "{source}");
        assert_eq!(frame.meta.html, first, "{source}");
        assert!(frame.meta.text_bindings.is_empty(), "{source}");
        assert!(frame.meta.events.is_empty(), "{source}");
        assert!(compiler.blocks.is_empty(), "{source}");
    }
    Ok(())
}

#[test]
fn markup_dispatch_preserves_bindings_comments_blocks_and_events() -> Result<()> {
    let source = concat!(
        "\u{03bb} {{first}}<!-- {{ignored}} -->",
        "<if\ncondition=\"ready\"><b>{{value}}</b></if>",
        "<for\neach=\"row in rows\"><i>{{row.name}}</i></for>",
        "{{{html}}}<button @click=\"{save()}\">{{tail}}</button>",
    );
    let meta = compile_to_metadata("dispatch-view", source, Vec::new())?;
    assert_eq!(meta.root.html, "<button></button>");
    assert_eq!(meta.root.conditionals[0].1, 0);
    assert_eq!(meta.root.repeats[0].block_index, 1);
    assert_eq!(meta.blocks.len(), 2);
    assert_eq!(meta.blocks[0].html, "<b></b>");
    assert_eq!(meta.blocks[1].html, "<i></i>");
    assert_eq!(meta.root.text_runs.len(), 3);
    assert_eq!(meta.root.text_runs[0].2, 2);
    assert_eq!(meta.root.text_runs[1].2, 1);
    assert_eq!(meta.root.text_runs[2].2, 0);
    assert!(matches!(
        &meta.root.text_runs[0].1[0],
        CompiledAttrPart::Static(value) if value == "\u{03bb} "
    ));
    assert!(matches!(
        &meta.root.text_runs[0].1[1],
        CompiledAttrPart::Dynamic(path) if path == "first"
    ));
    assert!(matches!(
        &meta.root.text_runs[1].1[0],
        CompiledAttrPart::Dynamic(path) if path == "html"
    ));
    assert!(matches!(
        &meta.root.text_runs[2].1[0],
        CompiledAttrPart::Dynamic(path) if path == "tail"
    ));
    assert_eq!(meta.root.events.len(), 1);
    assert_eq!(meta.root.events[0].0, "click");
    assert_eq!(meta.root.events[0].1, "save");
    assert_eq!(meta.root.event_targets, vec![1]);
    Ok(())
}

#[test]
fn markup_dispatch_still_validates_after_text_and_in_unused_declarations() {
    for source in [
        "prefix<for each=\"bad\">invalid</for>suffix",
        "<fragment name=\"unused\">prefix<for each=\"bad\">invalid</for></fragment>",
    ] {
        let error = generate_compiled_template("invalid-dispatch-view", source)
            .expect_err("literal dispatch must not bypass directive validation");
        assert!(matches!(
            error,
            crate::ParserError::Template(diagnostic)
                if diagnostic.error_code() == Some(codes::INVALID_FOR_EACH)
        ));
    }
}

#[test]
fn suspended_frames_preserve_deep_block_order_and_resume_root() -> Result<()> {
    let source = concat!(
        "<if condition=\"ready\"><for each=\"row in rows\"><if condition=\"row.ready\">",
        "<for each=\"child in row.children\"><if condition=\"child.ready\">",
        "<span>{{child.name}}</span></if></for></if></for></if><p>tail</p>",
    );
    let meta = compile_to_metadata("nested-view", source, Vec::new())?;
    assert_eq!(meta.root.html, "<p>tail</p>");
    assert_eq!(meta.blocks.len(), 5);
    assert_eq!(meta.root.conditionals[0].1, 0);
    assert_eq!(meta.blocks[0].repeats[0].block_index, 1);
    assert_eq!(meta.blocks[1].conditionals[0].1, 2);
    assert_eq!(meta.blocks[2].repeats[0].block_index, 3);
    assert_eq!(meta.blocks[3].conditionals[0].1, 4);
    assert_eq!(meta.blocks[4].text_runs.len(), 1);
    Ok(())
}

#[test]
fn suspended_cursors_exclude_section_metadata() {
    assert_eq!(
        std::mem::size_of::<SectionFrame>(),
        std::mem::size_of::<SectionCursor>() + std::mem::size_of::<TemplateSectionMeta>()
    );
    assert!(std::mem::size_of::<SectionCursor>() < std::mem::size_of::<SectionFrame>());
}

#[test]
fn parked_parent_metadata_survives_nested_siblings() -> Result<()> {
    let source = concat!(
        "{{before}}<if condition=\"left\">{{parent}}<for each=\"row in rows\">",
        "<span key=\"{{row.id}}\">{{row.name}}</span></for>{{afterParent}}</if>",
        "{{middle}}<if condition=\"right\">{{sibling}}</if>{{tail}}",
    );
    let meta = compile_to_metadata("siblings-view", source, Vec::new())?;
    assert_eq!(meta.blocks.len(), 3);
    assert_eq!(meta.root.conditionals[0].1, 0);
    assert_eq!(meta.root.conditionals[1].1, 2);
    assert_eq!(meta.blocks[0].repeats[0].block_index, 1);
    assert_eq!(meta.blocks[0].repeats[0].key_path.as_deref(), Some("id"));
    for (index, expected) in ["before", "middle", "tail"].into_iter().enumerate() {
        assert!(matches!(
            &meta.root.text_runs[index].1[0],
            CompiledAttrPart::Dynamic(path) if path == expected
        ));
    }
    for (index, expected) in ["parent", "afterParent"].into_iter().enumerate() {
        assert!(matches!(
            &meta.blocks[0].text_runs[index].1[0],
            CompiledAttrPart::Dynamic(path) if path == expected
        ));
    }
    assert!(matches!(
        &meta.blocks[2].text_runs[0].1[0],
        CompiledAttrPart::Dynamic(path) if path == "sibling"
    ));
    Ok(())
}

#[test]
fn parked_slots_preserve_recursive_declarations_and_unused_validation() -> Result<()> {
    let source = concat!(
        "<render fragment=\"a\" scope=\"rows\" as=\"items\"/>",
        "<fragment name=\"a\">{{ownerA}}<if condition=\"ready\"><for each=\"item in items\">",
        "<render fragment=\"b\" scope=\"item\" as=\"node\"/></for></if>{{afterA}}</fragment>",
        "<fragment name=\"b\">{{node.name}}<if condition=\"node.children.length\">",
        "<render fragment=\"a\" scope=\"node.children\" as=\"items\"/></if></fragment>",
        "<fragment name=\"unused\"><p>unused</p></fragment>",
    );
    let meta = compile_to_metadata("recursive-view", source, Vec::new())?;
    assert_eq!(meta.declaration_count, 3);
    assert_eq!(meta.blocks.len(), 6);
    assert_eq!(meta.root.renders[0].block_index, 0);
    assert_eq!(meta.blocks[0].conditionals[0].1, 3);
    assert_eq!(meta.blocks[3].repeats[0].block_index, 4);
    assert_eq!(meta.blocks[4].renders[0].block_index, 1);
    assert_eq!(meta.blocks[1].conditionals[0].1, 5);
    assert_eq!(meta.blocks[5].renders[0].block_index, 0);
    assert!(matches!(
        &meta.blocks[0].text_runs[1].1[0],
        CompiledAttrPart::Dynamic(path) if path == "afterA"
    ));
    let invalid = source.replace("<p>unused</p>", "<for each=\"bad\">invalid</for>");
    let error = generate_compiled_template("recursive-view", &invalid)
        .expect_err("unused declarations still validate");
    assert!(matches!(
        error,
        crate::ParserError::Template(diagnostic)
            if diagnostic.error_code() == Some(codes::INVALID_FOR_EACH)
    ));
    Ok(())
}
