// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;

fn successors(section: &TemplateSectionMeta) -> Vec<usize> {
    section.text_runs.iter().map(|entry| entry.2).collect()
}

#[test]
fn complete_successors_cover_every_dynamic_kind_and_actual_end() -> Result<()> {
    let source = concat!(
        r#"{{a}}<if condition="show">yes</if>{{b}}"#,
        r#"<for each="item in items">{{item}}</for>{{c}}"#,
        r#"<render fragment="part"/>{{d}}{{{html}}}{{e}}<span></span>{{tail}}"#,
        r#"<fragment name="part"></fragment>"#,
    );
    let meta = compile_to_metadata("successors", source, Vec::new())?;
    assert_eq!(meta.root.html, "<span></span>");
    assert_eq!(successors(&meta.root), [2, 3, 4, 5, 1, 6, 0]);
    let payload =
        generate_compiled_template_with_root_source("successors", source, source, false, false)?;
    assert!(payload.template_json.contains(
        r#""tx":[[[0,0],[["a"]],2],[[0,0,2],[["b"]],3],[[0,0,4],[["c"]],4],[[0,0,6],[["d"]],5],[[0,0,7],[["html"]],1],[[0,0,8],[["e"]],6],[[0,1],[["tail"]]]]"#
    ));
    assert!(!payload.template_json.contains("null"));
    Ok(())
}

#[test]
fn element_successors_use_section_indexes_not_sibling_ordinals() -> Result<()> {
    let meta = compile_to_metadata(
        "successors",
        "<section><i></i>{{nested}}<b><u></u></b>{{nestedEnd}}</section>{{root}}<footer></footer>",
        Vec::new(),
    )?;
    assert_eq!(successors(&meta.root), [22, 0, 38]);
    assert_eq!(meta.root.text_runs[0].0.parent_index, 1);
    assert_eq!(meta.root.text_runs[1].0.parent_index, 1);
    assert_eq!(meta.root.text_runs[2].0.parent_index, 0);
    Ok(())
}

#[test]
fn raw_successors_count_ranges_across_parents_without_numbering_raw_descendants() -> Result<()> {
    let meta = compile_to_metadata(
        "successors",
        "<div>{{{inside}}}</div>{{before}}{{{outside}}}{{last}}",
        Vec::new(),
    )?;
    assert_eq!(meta.root.html, "<div></div>");
    assert_eq!(successors(&meta.root), [1, 13, 1, 0]);
    Ok(())
}

#[test]
fn structural_successors_keep_section_local_indexes_across_parent_depths() -> Result<()> {
    let meta = compile_to_metadata(
        "successors",
        concat!(
            r#"<div><if condition="show"></if><for each="item in items"></for></div>"#,
            r#"{{a}}<if condition="show"></if>{{b}}<for each="item in items"></for>"#,
        ),
        Vec::new(),
    )?;
    assert_eq!(successors(&meta.root), [10, 11]);
    assert_eq!(meta.root.condition_slots[0].parent_index, 1);
    assert_eq!(meta.root.condition_slots[1].parent_index, 0);
    assert_eq!(meta.root.repeat_slots[0].parent_index, 1);
    assert_eq!(meta.root.repeat_slots[1].parent_index, 0);
    Ok(())
}

#[test]
fn implied_table_parents_and_nested_sections_keep_independent_successors() -> Result<()> {
    let meta = compile_to_metadata(
        "successors",
        concat!(
            r#"<table><tr><td>{{cell}}<b></b></td></tr></table>"#,
            r#"{{before}}<if condition="show">{{inner}}<i></i>{{end}}</if>"#,
            r#"{{repeat}}<for each="row in rows">{{row}}<u></u></for>"#,
        ),
        Vec::new(),
    )?;
    assert_eq!(
        meta.root.html,
        "<table><tbody><tr><td><b></b></td></tr></tbody></table>"
    );
    assert_eq!(meta.root.text_runs[0].0.parent_index, 4);
    assert_eq!(successors(&meta.root), [38, 2, 3]);
    assert_eq!(successors(&meta.blocks[0]), [6, 0]);
    assert_eq!(successors(&meta.blocks[1]), [6]);
    Ok(())
}

#[test]
fn authored_comments_in_finalizer_have_parent_local_successors_including_empty_comments() {
    // The source scanner strips ordinary comments. The finalizer also accepts
    // retained comments in static HTML; manual metadata exercises that contract.
    let mut section = TemplateSectionMeta::default();
    section.html.push_str("<!--first--><span>");
    section.text_bindings.push(("before".into(), false));
    emit_text_marker(&mut section, 0);
    section.html.push_str("<!---->");
    section.text_bindings.push(("after".into(), false));
    emit_text_marker(&mut section, 1);
    section.html.push_str("<!--note--></span>");
    section.text_bindings.push(("tail".into(), false));
    emit_text_marker(&mut section, 2);
    section.html.push_str("<!--last-->");
    finalize_template_section(&mut section);
    assert_eq!(
        section.html,
        "<!--first--><span><!----><!--note--></span><!--last-->"
    );
    assert_eq!(successors(&section), [7, 15, 15]);
    assert_eq!(section.text_runs[0].0.parent_index, 1);
    assert_eq!(section.text_runs[1].0.parent_index, 1);
    assert_eq!(section.text_runs[2].0.parent_index, 0);
}

#[test]
fn stripped_comments_merge_adjacent_parts_without_creating_a_missing_successor() -> Result<()> {
    let meta = compile_to_metadata(
        "successors",
        "<span>{{before}}<!---->{{after}}</span>",
        Vec::new(),
    )?;
    assert_eq!(meta.root.html, "<span></span>");
    assert_eq!(successors(&meta.root), [0]);
    assert_eq!(meta.root.text_runs[0].1.len(), 2);
    Ok(())
}

#[test]
fn slot_orders_reset_only_when_static_child_offsets_advance() -> Result<()> {
    let meta = compile_to_metadata(
        "successors",
        concat!(
            r#"{{a}}<if condition="show"></if>{{b}}<i></i>"#,
            r#"{{c}}<for each="item in items"></for>{{d}}<b></b>{{e}}"#,
        ),
        Vec::new(),
    )?;
    let slots: Vec<_> = meta
        .root
        .text_runs
        .iter()
        .map(|entry| (entry.0.before_index, entry.0.order))
        .collect();
    assert_eq!(slots, [(0, 0), (0, 2), (1, 0), (1, 2), (2, 0)]);
    assert_eq!(successors(&meta.root), [2, 6, 3, 14, 0]);
    Ok(())
}

#[test]
fn pruning_remaps_block_targets_but_not_successor_slot_indexes() -> Result<()> {
    let source = concat!(
        r#"<fragment name="unused"><b>never</b></fragment>"#,
        r#"<fragment name="a">{{a}}<render fragment="b"/></fragment>"#,
        r#"<fragment name="b">{{b}}<render fragment="a"/></fragment>"#,
        r#"<render fragment="a"/>{{middle}}<render fragment="b"/>"#,
    );
    let payload =
        generate_compiled_template_with_root_source("successors", source, source, false, false)?;
    assert!(payload
        .template_json
        .contains(r#""tx":[[[0,0,1],[["middle"]],12]]"#));
    assert!(payload
        .template_json
        .contains(r#""u":[[0,[0,0]],[1,[0,0,2]]]"#));
    assert!(payload
        .template_json
        .contains(r#""tx":[[[0,0],[["a"]],4]]"#));
    assert!(payload
        .template_json
        .contains(r#""tx":[[[0,0],[["b"]],4]]"#));
    assert!(!payload.template_json.contains("never"));
    Ok(())
}
