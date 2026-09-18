// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;
use webui_protocol::{FragmentList, WebUIFragmentAttribute};
use webui_test_utils::test_json;

crate::define_string_response_writer!(IndexWriter, output);

#[test]
fn adjacent_name_offsets_preserve_empty_skipped_mixed_and_last_attributes() -> Result<()> {
    let wire = WebUIProtocol::new(HashMap::from([
        (
            "a-first".to_owned(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<x"),
                    WebUIFragment::attribute("first-name", "value"),
                    WebUIFragment::attribute("", ""),
                    WebUIFragment {
                        fragment: Some(Fragment::Attribute(WebUIFragmentAttribute {
                            name: "ignored".to_owned(),
                            template: "m-value".to_owned(),
                            attr_skip: true,
                            ..Default::default()
                        })),
                    },
                    WebUIFragment::attribute_template("data-label", "m-value"),
                    WebUIFragment::attribute("données-prénom", "value"),
                    WebUIFragment::raw("></x>"),
                ],
                ..Default::default()
            },
        ),
        ("b-empty".to_owned(), FragmentList::default()),
        (
            "m-value".to_owned(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("mixed")],
                ..Default::default()
            },
        ),
        (
            "z-last".to_owned(),
            FragmentList {
                fragments: vec![WebUIFragment::attribute("last-name", "value")],
                ..Default::default()
            },
        ),
    ]));
    let protocol = Protocol::from_protobuf(&wire.to_protobuf()?)?;
    let resolved = protocol.render_fragments().resolve(protocol.protocol());
    let entry = resolved
        .list_by_id("a-first")
        .unwrap_or_else(|| panic!("entry should be prepared"));
    for (index, expected) in [
        (0, None),
        (1, Some("firstName")),
        (2, Some("")),
        (3, None),
        (4, Some("dataLabel")),
        (5, Some("donnéesPrénom")),
        (6, None),
        (7, None),
    ] {
        assert_eq!(entry.component_attr_name(index), expected);
    }
    assert_eq!(entry.target(3), resolved.index("m-value"));
    assert_eq!(entry.target(4), resolved.index("m-value"));
    assert_eq!(entry.target(entry.fragments.len()), None);
    let last = resolved
        .list_by_id("z-last")
        .unwrap_or_else(|| panic!("last record should be prepared"));
    assert_eq!(last.component_attr_name(0), Some("lastName"));
    let empty = resolved
        .list_by_id("b-empty")
        .unwrap_or_else(|| panic!("empty record should be prepared"));
    assert!(empty.fragments.is_empty());
    assert!(empty.metadata.is_empty());
    assert_eq!(empty.target(0), None);
    assert_eq!(empty.component_attr_name(0), None);
    let prepared = protocol.render_fragments().records.get();
    assert_eq!(prepared.metadata.len(), 10);
    assert_eq!(
        prepared.attr_names.as_ref(),
        "firstNamedataLabeldonnéesPrénomlastName"
    );
    assert_eq!(last.next_attr_start as usize, prepared.attr_names.len());
    let mut writer = IndexWriter::with_capacity(128);
    WebUIHandler::new().render(
        &protocol,
        &test_json!({"value": "V"}),
        &RenderOptions::new("a-first", "/"),
        &mut writer,
    )?;
    assert_eq!(
        writer.output,
        "<x first-name=\"V\" ignored=\"mixed\" data-label=\"mixed\" données-prénom=\"V\"></x>"
    );
    Ok(())
}

#[test]
fn fragment_ids_have_one_array_and_stable_shared_lookup_backing() {
    let protocol = Protocol::new(WebUIProtocol::new(
        ["z-last", "entry", "a-first"]
            .into_iter()
            .map(|id| {
                (
                    id.to_owned(),
                    FragmentList {
                        fragments: vec![WebUIFragment::raw(id)],
                        ..Default::default()
                    },
                )
            })
            .collect(),
    ));
    let index = protocol.render_fragments();
    assert_eq!(index.record_count(), 3);
    for (slot, id) in (0u32..).zip(index.ids.iter()) {
        assert_eq!(Arc::strong_count(id), 2);
        assert_eq!(index.index(id), Some(slot as usize));
        assert_eq!(index.id(slot as usize), Some(id.as_ref()));
        assert_eq!(protocol.fragment_slot(id), Some(slot));
        assert!(std::ptr::eq(
            protocol
                .fragment_id(slot)
                .unwrap_or_else(|| panic!("fragment slot should resolve")),
            id.as_ref()
        ));
    }
    assert_eq!(index.id(usize::MAX), None);
    assert_eq!(protocol.fragment_id(u32::MAX), None);
    let first = index.resolve(protocol.protocol());
    let second = index.resolve(protocol.protocol());
    assert!(std::ptr::eq(
        first
            .list(0)
            .unwrap_or_else(|| panic!("record should exist")),
        second
            .list(0)
            .unwrap_or_else(|| panic!("record should exist"))
    ));
    let first = first.view(0).unwrap_or_else(|| panic!("view should exist"));
    let second = second
        .view(0)
        .unwrap_or_else(|| panic!("view should exist"));
    assert!(std::ptr::eq(first.fragments, second.fragments));
    assert!(std::ptr::eq(first.metadata, second.metadata));
    assert!(std::ptr::eq(first.attr_names, second.attr_names));
}

#[test]
fn empty_protocol_records_share_the_terminal_metadata() {
    let protocol = Protocol::new(WebUIProtocol::new(HashMap::from([
        ("a".to_owned(), FragmentList::default()),
        ("b".to_owned(), FragmentList::default()),
    ])));
    let index = protocol.render_fragments();
    assert_eq!(index.records.get().metadata.len(), 1);
    let resolved = index.resolve(protocol.protocol());
    for slot in 0..2 {
        let view = resolved
            .view(slot)
            .unwrap_or_else(|| panic!("empty record should resolve"));
        assert!(view.fragments.is_empty());
        assert!(view.metadata.is_empty());
        assert!(view.attr_names.is_empty());
        assert_eq!(view.target(0), None);
    }
    assert!(resolved.view(2).is_none());
    assert!(resolved.view(usize::MAX).is_none());
    let empty = Protocol::new(WebUIProtocol::new(HashMap::new()));
    assert_eq!(empty.render_fragments().record_count(), 0);
    assert!(empty.render_fragments().records.get().metadata.is_empty());
    assert!(empty
        .render_fragments()
        .resolve(empty.protocol())
        .view(0)
        .is_none());
}

#[test]
fn adjacent_name_offsets_preserve_the_full_u32_length_range() {
    let metadata = RenderFragmentMetadata {
        target: NO_RENDER_SLOT,
        attr_start: u32::MAX - 10,
    };
    let start = metadata.attr_start as usize;
    assert_eq!(
        metadata.name_range(5),
        start.checked_add(16).map(|end| start..end)
    );
    assert_eq!(metadata.name_range(metadata.attr_start), Some(start..start));
    let reserved = RenderFragmentMetadata {
        attr_start: NO_ATTR_NAME,
        ..metadata
    };
    assert_eq!(reserved.name_range(0), None);
}

#[test]
fn absent_render_omits_capture_paths_even_with_dotted_shared_inputs() {
    let mut wire = WebUIProtocol::new(HashMap::from([
        (
            "entry".to_owned(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::attribute("value", "row.name"),
                    WebUIFragment::for_loop("row", "rows.children", "body"),
                ],
                ..Default::default()
            },
        ),
        ("body".to_owned(), FragmentList::default()),
    ]));
    wire.components
        .insert("entry".to_owned(), webui_protocol::ComponentData::default());
    let protocol = Protocol::new(wire);
    let index = protocol.render_fragments();
    assert!(index.capture_paths.is_none());
    assert_eq!(index.provenance_policy(), state_view::Provenance::Omit);
    assert_eq!(
        index.resolve(protocol.protocol()).provenance_policy(),
        state_view::Provenance::Omit
    );
    assert_eq!(
        std::mem::size_of::<Option<HashSet<Arc<str>>>>(),
        std::mem::size_of::<HashSet<Arc<str>>>()
    );
}

#[test]
fn unreachable_parameterless_render_requires_tracking_with_an_empty_path_dictionary() {
    let protocol = Protocol::new(WebUIProtocol::new(HashMap::from([
        (
            "entry".to_owned(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("ordinary")],
                ..Default::default()
            },
        ),
        (
            "unused".to_owned(),
            FragmentList {
                fragments: vec![WebUIFragment::render("body", "", "")],
                ..Default::default()
            },
        ),
        ("body".to_owned(), FragmentList::default()),
    ])));
    let index = protocol.render_fragments();
    assert!(index.capture_paths.as_ref().is_some_and(HashSet::is_empty));
    assert_eq!(index.provenance_policy(), state_view::Provenance::Track);
    assert_eq!(
        index.resolve(protocol.protocol()).provenance_policy(),
        state_view::Provenance::Track
    );
}

#[test]
fn unreachable_render_prepares_projection_suffixes_for_every_shared_input() {
    let protocol = Protocol::new(WebUIProtocol::new(HashMap::from([
        (
            "entry".to_owned(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::attribute("value", "row.name"),
                    WebUIFragment::for_loop("row", "rows.children", "body"),
                ],
                ..Default::default()
            },
        ),
        (
            "unused".to_owned(),
            FragmentList {
                fragments: vec![WebUIFragment::render("body", "row.profile", "row")],
                ..Default::default()
            },
        ),
        ("body".to_owned(), FragmentList::default()),
    ])));
    let index = protocol.render_fragments().resolve(protocol.protocol());
    assert_eq!(index.provenance_policy(), state_view::Provenance::Track);
    for suffix in ["name", "children", "profile"] {
        assert!(Arc::ptr_eq(
            &index.capture_path(suffix),
            &index.capture_path(suffix)
        ));
    }
}
