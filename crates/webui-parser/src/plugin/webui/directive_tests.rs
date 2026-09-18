// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use super::*;
use crate::directive_expression_tests::RECURSIVE_TREE;
use crate::ParserError;

#[test]
fn fragment_nodes_own_strings_and_marker_indices_after_input_drop() {
    let raw = String::from("<p title=\"kept\">before<!--t:0-->after<!--literal--></p>");
    let offsets = vec![raw.find("<!--t:0-->").expect("marker")];
    let nodes = parse_fragment_nodes(&raw, &offsets);
    drop(raw);
    drop(offsets);
    let [FragmentNode::Element(element)] = nodes.as_slice() else {
        panic!("expected owned element");
    };
    assert_eq!(element.tag_name, "p");
    assert_eq!(element.attrs[0].name, "title");
    assert_eq!(element.attrs[0].value.as_deref(), Some("kept"));
    assert!(matches!(
        element.children.as_slice(),
        [
            FragmentNode::Text(before),
            FragmentNode::TextMarker(0),
            FragmentNode::Text(after),
            FragmentNode::Comment(comment),
        ] if before == "before" && after == "after" && comment == "literal"
    ));
}

#[test]
fn finalization_consumes_intermediate_text_bindings() -> Result<()> {
    let source = "<span>{{root}}</span><if condition=\"ready\"><b>{{child}}</b></if>";
    let meta = compile_to_metadata("text-view", source, Vec::new())?;
    assert!(meta.root.text_bindings.is_empty());
    assert!(meta
        .blocks
        .iter()
        .all(|block| block.text_bindings.is_empty()));
    assert!(matches!(
        &meta.root.text_runs[0].1[0],
        CompiledAttrPart::Dynamic(path) if path == "root"
    ));
    assert!(matches!(
        &meta.blocks[0].text_runs[0].1[0],
        CompiledAttrPart::Dynamic(path) if path == "child"
    ));
    Ok(())
}

#[test]
fn issue_518_whole_braced_tree_compiles_to_one_recursive_body() -> Result<()> {
    let meta = compile_to_metadata("tree-view", RECURSIVE_TREE, Vec::new())?;
    assert_eq!(meta.declaration_count, 1);
    assert_eq!(meta.blocks.len(), 3);
    assert!(meta.root.html.is_empty());
    assert_eq!(meta.root.renders[0].block_index, 0);
    assert_eq!(meta.blocks[0].repeats[0].item_var, "child");
    assert_eq!(meta.blocks[0].repeats[0].collection, "items");
    assert_eq!(meta.blocks[0].repeats[0].block_index, 1);
    assert_eq!(
        meta.blocks[1].conditionals[0].0,
        ConditionExpr::identifier("child.children.length")
    );
    assert_eq!(meta.blocks[1].conditionals[0].1, 2);
    assert_eq!(meta.blocks[2].renders[0].block_index, 0);
    assert_eq!(meta.blocks[2].renders[0].scope, "child.children");
    let payload = tests::generate_compiled_template_payload("tree-view", RECURSIVE_TREE);
    assert_eq!(payload.hydration_roots, ["items"]);
    assert!(!payload.template_json.contains("<fragment"));
    assert!(!payload.template_json.contains("<render"));
    Ok(())
}

#[test]
fn wrapped_directives_preserve_keys_conditions_and_dependency_roots() {
    let source = concat!(
        "<for each=\"group in groups\"><if condition=\"group.visible && ready\">",
        "<for each=\"child in group.children\"><span key=\"{{child.id}}\">",
        "{{child.name}} {{ownerTitle}}</span></for></if></for>",
    );
    let wrapped = source
        .replace("each=\"group in groups\"", "each=\"{{ group in groups }}\"")
        .replace(
            "condition=\"group.visible && ready\"",
            "condition=\"{{ group.visible && ready }}\"",
        )
        .replace(
            "each=\"child in group.children\"",
            "each=\"{{ child in group.children }}\"",
        );
    let bare = tests::generate_compiled_template_payload("nested-view", source);
    let normalized = tests::generate_compiled_template_payload("nested-view", &wrapped);
    assert_eq!(normalized.template_json, bare.template_json);
    assert_eq!(normalized.template_functions, bare.template_functions);
    assert_eq!(normalized.hydration_roots, bare.hydration_roots);
    assert!(normalized
        .hydration_roots
        .iter()
        .any(|root| root == "groups"));
    assert!(normalized
        .hydration_roots
        .iter()
        .any(|root| root == "ready"));
    assert!(!normalized
        .hydration_roots
        .iter()
        .any(|root| root == "child" || root == "group"));
}

#[test]
fn wrapped_conditions_preserve_quoted_braces_and_attribute_quotes() -> Result<()> {
    let source = "<if condition='{{ ready }}'><if condition=\"{{ label == '}}' }}\">yes</if></if>";
    let bare = "<if condition='ready'><if condition=\"label == '}}'\">yes</if></if>";
    assert_eq!(
        generate_compiled_template("quoted-view", source)?,
        generate_compiled_template("quoted-view", bare)?
    );
    Ok(())
}

#[test]
fn public_compiler_rejects_malformed_control_wrappers_at_nested_locations() {
    for (element, attribute, expression, code) in [
        (
            "for",
            "each",
            "{{{child in items}}}",
            codes::INVALID_FOR_EACH,
        ),
        ("for", "each", "{{child in items}", codes::INVALID_FOR_EACH),
        (
            "for",
            "each",
            "{{child in items}} {{other}}",
            codes::INVALID_FOR_EACH,
        ),
        ("for", "each", "{{child}}", codes::INVALID_FOR_EACH),
        (
            "for",
            "each",
            "{{child in items[0]}}",
            codes::INVALID_FOR_IDENTIFIER,
        ),
        (
            "if",
            "condition",
            "{{{ready}}}",
            codes::INVALID_IF_CONDITION,
        ),
        ("if", "condition", "{{ready}", codes::INVALID_IF_CONDITION),
        (
            "if",
            "condition",
            "{{ready}}tail",
            codes::INVALID_IF_CONDITION,
        ),
        (
            "if",
            "condition",
            "{{ready}} && {{other}}",
            codes::INVALID_IF_CONDITION,
        ),
    ] {
        let source = format!(
            "<div>\n<if condition=\"ready\">\n<{element} {attribute}=\"{expression}\">bad</{element}>\n</if></div>"
        );
        let error = generate_compiled_template("invalid-view", &source).expect_err(&source);
        let ParserError::Template(diagnostic) = error else {
            panic!("expected template diagnostic");
        };
        assert_eq!(diagnostic.error_code(), Some(code), "{expression}");
        assert_eq!(diagnostic.component_name(), Some("invalid-view"));
        assert_eq!(diagnostic.position_line_column(), Some((3, 1)));
        assert!(!diagnostic.to_string().contains('\x1b'));
    }
}
