// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::tests::{assert_no_client_markers, generate_compiled_template_payload};
use super::*;

#[test]
fn recursive_fragment_compiles_once_with_caller_evaluated_scope() -> Result<()> {
    let source = concat!(
        r#"<render fragment="tree-items" scope="{{treeData}}" as="items"/>"#,
        r#"<fragment name="tree-items"><for each="child in items">"#,
        r#"<li key="{{child.id}}" title="{{ownerTitle}}">{{child.label}}"#,
        r#"<render fragment="tree-items" scope="{{child.children}}" as="items"/>"#,
        "</li></for></fragment>",
    );
    let meta = compile_to_metadata("tree-view", source, Vec::new())?;
    assert_eq!(meta.blocks.len(), 2);
    assert_eq!(meta.root.renders[0].block_index, 0);
    assert_eq!(meta.blocks[0].repeats[0].block_index, 1);
    assert_eq!(meta.blocks[0].repeats[0].key_path.as_deref(), Some("id"));
    assert_eq!(meta.blocks[1].renders[0].block_index, 0);
    let payload = generate_compiled_template_payload("tree-view", source);
    assert_no_client_markers(&payload.template_json);
    assert_eq!(payload.hydration_roots, ["ownerTitle", "treeData"]);
    assert!(payload
        .template_json
        .contains(r#""u":[[0,[0,0],"treeData","items"]]"#));
    assert!(payload
        .template_json
        .contains(r#""child.children","items"]"#));
    assert!(!payload.template_json.contains("key="));
    assert!(!payload.template_json.contains("<fragment"));
    assert!(!payload.template_json.contains("<render"));
    Ok(())
}

#[test]
fn mutual_forward_calls_use_a_flat_block_table() -> Result<()> {
    let source = concat!(
        r#"<render fragment="first"/>"#,
        r#"<fragment name="first">{{firstOwner}}<render fragment="second"/></fragment>"#,
        r#"<fragment name="second">{{secondOwner}}<render fragment="first"/></fragment>"#,
    );
    let meta = compile_to_metadata("mutual-view", source, Vec::new())?;
    assert_eq!(meta.blocks.len(), 2);
    assert_eq!(meta.blocks[0].renders[0].block_index, 1);
    assert_eq!(meta.blocks[1].renders[0].block_index, 0);
    let payload = generate_compiled_template_payload("mutual-view", source);
    assert_eq!(payload.hydration_roots, ["firstOwner", "secondOwner"]);
    assert_eq!(payload.template_json.matches(r#""b":["#).count(), 1);
    assert!(payload.template_json.contains(r#""u":[[0,[0,0]]]"#));
    Ok(())
}

#[test]
fn parameterless_call_retains_owner_root_even_when_other_calls_bind_it() {
    let source = concat!(
        r#"<render fragment="part" scope="data" as="items"/><render fragment="part"/>"#,
        r#"<fragment name="part">{{items}} {{title}}</fragment>"#,
    );
    let payload = generate_compiled_template_payload("owner-view", source);
    assert_eq!(payload.hydration_roots, ["data", "items", "title"]);
    assert!(payload
        .template_json
        .contains(r#""u":[[0,[0,0],"data","items"],[0,[0,0,1]]]"#));
}

#[test]
fn differing_callsite_aliases_union_free_owner_roots() {
    let source = concat!(
        r#"<render fragment="part" scope="left" as="foo"/>"#,
        r#"<render fragment="part" scope="right" as="bar"/>"#,
        r#"<fragment name="part">{{foo.value}} {{bar.value}} {{owner}}</fragment>"#,
    );
    let payload = generate_compiled_template_payload("alias-view", source);
    assert_eq!(
        payload.hydration_roots,
        ["bar", "foo", "left", "owner", "right"]
    );
}

#[test]
fn callee_owner_reads_are_not_filtered_by_caller_loop_or_alias() {
    let source = concat!(
        r#"<for each="foo in rows"><render fragment="caller" scope="foo" as="bar"/></for>"#,
        r#"<fragment name="caller">{{bar.name}}<render fragment="callee"/></fragment>"#,
        r#"<fragment name="callee">{{foo.name}} {{bar.name}}</fragment>"#,
    );
    let payload = generate_compiled_template_payload("scope-view", source);
    assert_eq!(payload.hydration_roots, ["bar", "foo", "rows"]);
}

#[test]
fn loop_shadowing_excludes_own_reads_but_not_called_owner_reads() {
    let source = concat!(
        r#"<render fragment="first" scope="data" as="items"/>"#,
        r#"<fragment name="first"><for each="items in rows">{{items.id}}"#,
        r#"<render fragment="second" scope="items.children" as="children"/></for></fragment>"#,
        r#"<fragment name="second">{{children.length}} {{items.owner}}</fragment>"#,
    );
    let payload = generate_compiled_template_payload("scope-view", source);
    assert_eq!(payload.hydration_roots, ["data", "items", "rows"]);
}

#[test]
fn cyclic_alias_projection_preserves_transitive_owner_reads() {
    let source = concat!(
        r#"<render fragment="first" scope="treeData" as="foo"/>"#,
        r#"<fragment name="first">{{foo.label}} {{bar.label}}"#,
        r#"<render fragment="second" scope="foo.children" as="bar"/></fragment>"#,
        r#"<fragment name="second">{{bar.label}} {{foo.label}}"#,
        r#"<render fragment="first" scope="bar.children" as="foo"/></fragment>"#,
    );
    let payload = generate_compiled_template_payload("cycle-view", source);
    assert_eq!(payload.hydration_roots, ["bar", "foo", "treeData"]);
}

#[test]
fn repeated_calls_grow_edges_not_declaration_bodies() -> Result<()> {
    let mut source = String::with_capacity(8192);
    for _ in 0..100 {
        source.push_str(r#"<render fragment="part"/>"#);
    }
    source.push_str(r#"<fragment name="part"><b>unique-body</b></fragment>"#);
    let meta = compile_to_metadata("size-view", &source, Vec::new())?;
    assert_eq!(meta.root.renders.len(), 100);
    assert_eq!(meta.blocks.len(), 1);
    let payload = generate_compiled_template_payload("size-view", &source);
    assert_eq!(payload.template_json.matches("unique-body").count(), 1);
    assert!(payload.template_json.len() < 2400);
    Ok(())
}

#[test]
fn unused_declarations_are_validated_before_pruning() {
    for (body, code) in [
        (
            r#"<button @click="{not valid}"></button>"#,
            codes::INVALID_EVENT_HANDLER,
        ),
        (r#"<div w-ref="notBraced"></div>"#, codes::INVALID_W_REF),
        (
            r#"<style w-ref="notBraced">b{color:red}</style>"#,
            codes::INVALID_W_REF,
        ),
        (
            r#"<style @click="{not valid}"></style>"#,
            codes::INVALID_EVENT_HANDLER,
        ),
        (r#"<div key="{{item.id}}"></div>"#, codes::INVALID_FOR_KEY),
        (
            r#"<for each="item in items"><div key="{{other.id}}"></div></for>"#,
            codes::INVALID_FOR_KEY,
        ),
    ] {
        let source = format!(r#"<p>root</p><fragment name="unused">{body}</fragment>"#);
        let result = super::generate_compiled_template("invalid-view", &source);
        let Err(error) = result else {
            panic!("unused invalid body must fail: {body}");
        };
        assert!(error.to_string().contains(code), "{error}");
    }
}

#[test]
fn scriptless_events_in_unused_declarations_are_rejected() {
    let source = concat!(
        "<p>root</p>",
        r#"<fragment name="unused"><if condition="enabled">"#,
        r#"<button @click="{submit()}"></button></if></fragment>"#,
    );
    let result =
        generate_compiled_template_with_root_source("scriptless-view", source, source, false, true);
    let Err(error) = result else {
        panic!("unused events still require a client module");
    };
    assert!(error.to_string().contains(codes::SCRIPTLESS_EVENT_HANDLER));
}

#[test]
fn unused_graphs_blocks_resources_and_condition_functions_are_pruned() -> Result<()> {
    let source = concat!(
        r#"<render fragment="live"/><if condition="rootVisible"><em>root</em></if>"#,
        r#"<fragment name="unused"><link rel="stylesheet" href="dead.css"/>"#,
        r#"<style>.unused { color: {{deadColor}}; }</style>"#,
        r#"<button @click="{deadHandler()}"></button><if condition="deadCondition">"#,
        r#"<for each="dead in deadRows">{{dead}}</for></if>"#,
        r#"<render fragment="unused"/></fragment>"#,
        r#"<fragment name="live"><if condition="liveVisible"><b>{{liveText}}</b></if></fragment>"#,
    );
    let mut meta = compile_to_metadata("prune-view", source, Vec::new())?;
    let build = fragment_graph::collect_build_metadata(&meta);
    assert!(build.has_events);
    assert_eq!(build.roots, ["liveText", "liveVisible", "rootVisible"]);
    fragment_graph::prune_unreachable(&mut meta);
    assert_eq!(meta.blocks.len(), 3);
    assert_eq!(meta.root.renders[0].block_index, 0);
    assert_eq!(meta.root.conditionals[0].1, 1);
    assert_eq!(meta.blocks[0].conditionals[0].1, 2);
    let payload = generate_compiled_template_payload("prune-view", source);
    assert_eq!(payload.template_functions.matches("function(").count(), 2);
    assert!(!payload.template_json.contains("dead"));
    assert!(!payload.template_json.contains("unused"));
    assert!(!payload.template_functions.contains("deadCondition"));
    Ok(())
}

#[test]
fn entirely_unused_cycles_emit_no_blocks_roots_or_functions() {
    let source = concat!(
        "<p>root</p>",
        r#"<fragment name="first"><if condition="dead">x</if><render fragment="second"/></fragment>"#,
        r#"<fragment name="second">{{unused}}<render fragment="first"/></fragment>"#,
    );
    let payload = generate_compiled_template_payload("prune-view", source);
    assert_eq!(payload.template_json, r#"{"h":"<p>root</p>"}"#);
    assert!(payload.hydration_roots.is_empty());
    assert!(payload.template_functions.is_empty());
}

#[test]
fn bare_and_shadow_root_wrappers_share_local_declarations() {
    for (attrs, event) in [
        ("", ""),
        (r#" shadowrootmode="open""#, r#" @click="{rootClick(e)}""#),
    ] {
        let source = format!(
            r#"<!-- lead --><template{attrs}{event}><render fragment="part"/><fragment name="part"><i>{{{{label}}}}</i></fragment></template>"#,
        );
        let payload = generate_compiled_template_payload("wrapper-view", &source);
        assert_no_client_markers(&payload.template_json);
        assert!(!payload.template_json.contains("<template"));
        if !event.is_empty() {
            assert!(payload
                .template_json
                .contains(r#""re":[["click","rootClick",[["e"]]]]"#));
        }
        assert_eq!(payload.hydration_roots, ["label"]);
    }
}

#[test]
fn boundary_stripping_cannot_hide_invalid_declaration_placement() {
    let source = r#"<boundary name="area"><fragment name="part">x</fragment></boundary>"#;
    let result = super::generate_compiled_template("placement-view", source);
    let Err(error) = result else {
        panic!("declaration inside boundary is not an owning-root child");
    };
    assert!(error
        .to_string()
        .contains(codes::INVALID_FRAGMENT_PLACEMENT));
}

#[test]
fn rendered_boundary_body_stays_wrapperless() {
    let source = concat!(
        r#"<render fragment="part"/>"#,
        r#"<fragment name="part"><boundary name="area"><b>{{label}}</b></boundary></fragment>"#,
    );
    let payload = generate_compiled_template_payload("boundary-view", source);
    assert!(!payload.template_json.contains("boundary"));
    assert!(payload.template_json.contains(r#""h":"<b></b>""#));
}

#[test]
fn call_slots_share_order_with_text_raw_if_and_repeat() -> Result<()> {
    let source = concat!(
        r#"{{before}}<render fragment="empty"/>{{{raw}}}"#,
        r#"<if condition="show">yes</if><for each="item in rows">{{item}}</for>"#,
        r#"<render fragment="empty"/>{{after}}"#,
        r#"<fragment name="empty"></fragment>"#,
    );
    let meta = compile_to_metadata("slot-view", source, Vec::new())?;
    assert_eq!(meta.root.html, "");
    assert_eq!(meta.root.text_runs[0].0.order, 0);
    assert_eq!(meta.root.render_slots[0].order, 1);
    assert_eq!(meta.root.text_runs[1].0.order, 2);
    assert_eq!(meta.root.condition_slots[0].order, 3);
    assert_eq!(meta.root.repeat_slots[0].order, 4);
    assert_eq!(meta.root.render_slots[1].order, 5);
    assert_eq!(meta.root.text_runs[2].0.order, 6);
    for slot in &meta.root.render_slots {
        assert_eq!((slot.parent_index, slot.before_index), (0, 0));
    }
    assert!(meta.blocks[0].html.is_empty());
    Ok(())
}

#[test]
fn static_text_around_calls_does_not_merge_across_the_insertion_slot() -> Result<()> {
    let source = concat!(
        r#"before<render fragment="part"/>after"#,
        r#"<fragment name="part">middle</fragment>"#,
    );
    let meta = compile_to_metadata("text-view", source, Vec::new())?;
    assert_eq!(meta.root.html, "");
    assert_eq!(meta.root.text_runs.len(), 2);
    assert_eq!(meta.root.text_runs[0].0.order, 0);
    assert_eq!(meta.root.render_slots[0].order, 1);
    assert_eq!(meta.root.text_runs[1].0.order, 2);
    assert_eq!(meta.blocks[0].html, "middle");
    Ok(())
}

#[test]
fn fragment_bodies_support_multiple_roots_text_raw_and_empty_content() -> Result<()> {
    let source = concat!(
        r#"<div><render fragment="multi"/></div><render fragment="empty"/>"#,
        r#"<fragment name="multi"><b>A</b>{{label}}<i>B</i>{{{html}}}</fragment>"#,
        r#"<fragment name="empty"><!-- empty --></fragment>"#,
    );
    let meta = compile_to_metadata("multi-view", source, Vec::new())?;
    assert_eq!(meta.root.html, "<div></div>");
    assert_eq!(meta.root.render_slots[0].parent_index, 1);
    assert_eq!(meta.root.render_slots[1].parent_index, 0);
    assert_eq!(meta.root.render_slots[1].before_index, 1);
    assert_eq!(meta.blocks[0].html, "<b>A</b><i>B</i>");
    assert_eq!(meta.blocks[0].text_runs[0].0.before_index, 1);
    assert_eq!(meta.blocks[0].text_runs[1].0.before_index, 2);
    assert_eq!(meta.blocks[0].text_runs[1].2, 1);
    assert!(meta.blocks[1].html.is_empty());
    Ok(())
}

#[test]
fn render_slots_follow_implied_table_containers() -> Result<()> {
    let source = concat!(
        r#"<table><tr><td>head</td></tr><render fragment="row"/></table>"#,
        r#"<button @click="{save()}">Save</button>"#,
        r#"<fragment name="row"><tr><td>{{value}}</td></tr></fragment>"#,
    );
    let meta = compile_to_metadata("table-view", source, Vec::new())?;
    assert_eq!(meta.root.render_slots[0].parent_index, 2);
    assert_eq!(meta.root.render_slots[0].before_index, 1);
    assert_eq!(meta.root.event_targets, [5]);
    assert!(meta
        .root
        .html
        .contains("<tbody><tr><td>head</td></tr></tbody>"));
    assert!(is_table_run_trivia(
        &FragmentNode::Comment("u:0".into()),
        true
    ));
    assert!(!is_table_run_trivia(
        &FragmentNode::Comment("u:0".into()),
        false
    ));
    Ok(())
}

#[test]
fn pure_bare_table_render_slots_have_compiler_owned_containers() -> Result<()> {
    for (body, container) in [
        ("<tr><td>{{value}}</td></tr>", "tbody"),
        ("<col>", "colgroup"),
    ] {
        let source = format!(
            "<fragment name=\"row\">{body}</fragment><table><render fragment=\"row\"/><render fragment=\"row\"/></table><button @click=\"{{save()}}\">save</button>"
        );
        let meta = compile_to_metadata("table-view", &source, Vec::new())?;
        assert_eq!(
            meta.root.html,
            format!("<table><{container}></{container}></table><button>save</button>")
        );
        assert_eq!(meta.root.render_slots[0].parent_index, 2);
        assert_eq!(meta.root.render_slots[1].parent_index, 2);
        assert_eq!(meta.root.render_slots[1].order, 1);
        assert_eq!(meta.root.event_targets, [3]);
    }
    Ok(())
}

#[test]
fn repeat_key_skip_survives_borrowed_if_body_ranges() -> Result<()> {
    let source = concat!(
        r#"<render fragment="rows" scope="data" as="items"/>"#,
        r#"<fragment name="rows"><for each="item in items">"#,
        "\n <!-- key trivia --> \n",
        r#"<if condition="item.visible">  <tr key="{{item.id}}" title="{{item.title}}">"#,
        r#"<td>{{item.label}}</td></tr> </if></for></fragment>"#,
    );
    let meta = compile_to_metadata("key-view", source, Vec::new())?;
    assert_eq!(meta.blocks[0].repeats[0].key_path.as_deref(), Some("id"));
    assert_eq!(meta.blocks[2].html, "<tr><td></td></tr>");
    assert_eq!(meta.blocks[2].attr_bindings.len(), 1);
    assert_eq!(meta.blocks[2].text_runs.len(), 1);
    Ok(())
}

#[test]
fn event_arguments_use_fragment_aliases_and_isolated_owner_roots() {
    let source = concat!(
        r#"<for each="owner in rows"><render fragment="part" scope="owner" as="item"/></for>"#,
        r#"<fragment name="part"><button @click="{save(e, item.id, owner.id, selected)}">Save</button>"#,
        r#"<for each="entry in item.children"><button @click="{pick(entry.id, suffix)}"></button></for>"#,
        "</fragment>",
    );
    let payload = generate_compiled_template_payload("event-view", source);
    assert_eq!(
        payload.hydration_roots,
        ["owner", "rows", "selected", "suffix"]
    );
    assert!(payload.template_json.contains(r#"["p","item.id"]"#));
    assert!(payload.template_json.contains(r#"["p","entry.id"]"#));
}

#[test]
fn deep_declaration_chain_compiles_without_graph_recursion() -> Result<()> {
    let count = 2048;
    let mut source = String::with_capacity(count * 90);
    source.push_str(r#"<render fragment="part0"/>"#);
    for index in 0..count {
        let _ = write!(source, r#"<fragment name="part{index}">"#);
        if index + 1 == count {
            source.push_str("{{leaf}}");
        } else {
            let _ = write!(source, r#"<render fragment="part{}"/>"#, index + 1);
        }
        source.push_str("</fragment>");
    }
    let meta = compile_to_metadata("deep-view", &source, Vec::new())?;
    assert_eq!(meta.blocks.len(), count);
    let payload = generate_compiled_template_payload("deep-view", &source);
    assert_eq!(payload.hydration_roots, ["leaf"]);
    assert_eq!(payload.template_json.matches(r#""h":"#).count(), count + 1);
    Ok(())
}

#[test]
fn nested_if_for_sections_keep_original_depth_first_block_order() -> Result<()> {
    let source = concat!(
        r#"<if condition="first"><for each="item in items"><if condition="item.ok">"#,
        "<b>deep</b></if></for></if>",
        r#"<for each="row in rows"><i>last</i></for>"#,
    );
    let meta = compile_to_metadata("ordinary-view", source, Vec::new())?;
    assert_eq!(meta.declaration_count, 0);
    assert_eq!(meta.root.conditionals[0].1, 0);
    assert_eq!(meta.blocks[0].repeats[0].block_index, 1);
    assert_eq!(meta.blocks[1].conditionals[0].1, 2);
    assert_eq!(meta.root.repeats[0].block_index, 3);
    assert_eq!(meta.blocks[2].html, "<b>deep</b>");
    assert_eq!(meta.blocks[3].html, "<i>last</i>");
    Ok(())
}
