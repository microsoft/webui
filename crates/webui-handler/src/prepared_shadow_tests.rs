// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;
use webui_protocol::{ComponentData, FragmentList};
use webui_test_utils::test_json;

crate::define_string_response_writer!(StyleWriter, output);

fn structural(value: &str) -> WebUIFragment {
    WebUIFragment::signal(format!("{STRUCTURAL_SIGNAL_PREFIX}{value}"), true)
}

fn list(fragments: Vec<WebUIFragment>) -> FragmentList {
    FragmentList {
        fragments,
        contains_boundary: false,
    }
}

fn styled_protocol() -> WebUIProtocol {
    let mut protocol = WebUIProtocol::new(HashMap::from([
        (
            "index.html".to_owned(),
            list(vec![
                WebUIFragment::raw("<html><head>"),
                structural("head_end"),
                WebUIFragment::raw("</head><body><light-child>"),
                WebUIFragment::component("light-child"),
                WebUIFragment::raw("</light-child><shadow-child>"),
                WebUIFragment::component("shadow-child"),
                WebUIFragment::raw("</shadow-child><shadow-child>"),
                WebUIFragment::component("shadow-child"),
                WebUIFragment::raw("</shadow-child></body></html>"),
            ]),
        ),
        (
            "light-child".to_owned(),
            list(vec![WebUIFragment::raw("<p class=\"light\">Light</p>")]),
        ),
        (
            "shadow-child".to_owned(),
            list(vec![
                WebUIFragment::raw("<template shadowrootmode=\"open\">"),
                structural("shadow_styles:shadow-child"),
                WebUIFragment::component("light-child"),
                WebUIFragment::raw("</template>"),
            ]),
        ),
    ]));
    protocol.components = HashMap::from([
        (
            "light-child".to_owned(),
            ComponentData {
                css: ".light{color:red}".to_owned(),
                ..Default::default()
            },
        ),
        (
            "shadow-child".to_owned(),
            ComponentData {
                css: ".shadow{color:blue}".to_owned(),
                uses_shadow_dom: true,
                ..Default::default()
            },
        ),
    ]);
    protocol.set_css_strategy(webui_protocol::CssStrategy::Style);
    protocol.populate_style_closures(&["index.html"]);
    protocol
}

fn descriptor(flags: u32) -> RenderFragmentList<'static> {
    static EMPTY: FragmentList = FragmentList {
        fragments: Vec::new(),
        contains_boundary: false,
    };
    RenderFragmentList {
        list: &EMPTY,
        metadata_start: 0,
        flags,
    }
}

#[test]
fn prepared_shadow_flags_fit_the_existing_descriptor_on_every_pointer_width() {
    fn require_copy<T: Copy>() {}
    require_copy::<RenderFragmentList<'_>>();
    require_copy::<RenderFragmentView<'_>>();
    assert_eq!(
        std::mem::size_of::<RenderFragmentList<'_>>(),
        std::mem::size_of::<(usize, u32, u32)>()
    );
    assert_eq!(
        std::mem::size_of::<RenderFragmentView<'_>>(),
        std::mem::size_of::<([usize; 6], bool)>()
    );
    assert_eq!(std::mem::size_of::<RenderFragmentMetadata>(), 8);
}

#[test]
fn prepared_shadow_flags_preserve_routes_light_components_and_index_zero() {
    let protocol = styled_protocol();
    let indices = HashMap::from([("shadow-child".to_owned(), 0)]);
    for has_routes in [false, true] {
        let shadow = descriptor(prepare_render_flags(
            &protocol,
            "shadow-child",
            &indices,
            has_routes,
        ));
        assert_eq!(shadow.has_routes(), has_routes);
        assert!(shadow.owns_css_tree());
        assert_eq!(shadow.shadow_style_index(), Some(0));
        for component in ["light-child", "index.html", "absent"] {
            let light = descriptor(prepare_render_flags(
                &protocol, component, &indices, has_routes,
            ));
            assert_eq!(light.has_routes(), has_routes);
            assert!(!light.owns_css_tree());
            assert_eq!(light.shadow_style_index(), None);
        }
    }
}

#[test]
fn prepared_shadow_flags_keep_unrepresentable_indices_on_the_fallible_path() {
    let protocol = styled_protocol();
    for index in [
        UNPREPARED_SHADOW_STYLE - 2,
        UNPREPARED_SHADOW_STYLE - 1,
        u32::MAX,
    ] {
        let indices = HashMap::from([("shadow-child".to_owned(), index)]);
        let shadow = descriptor(prepare_render_flags(
            &protocol,
            "shadow-child",
            &indices,
            true,
        ));
        assert!(shadow.has_routes());
        assert!(shadow.owns_css_tree());
        assert_eq!(
            shadow.shadow_style_index(),
            (index < UNPREPARED_SHADOW_STYLE - 1).then_some(index)
        );
    }
}

#[test]
fn prepared_shadow_flags_do_not_hide_missing_closures_or_indices() {
    let mut protocol = styled_protocol();
    let indices = HashMap::from([("shadow-child".to_owned(), 0)]);
    let missing_index = HashMap::new();
    for missing_closure in [false, true] {
        if missing_closure {
            protocol.style_closures.remove("shadow-child");
        }
        let index = if missing_closure {
            &indices
        } else {
            &missing_index
        };
        let shadow = descriptor(prepare_render_flags(
            &protocol,
            "shadow-child",
            index,
            false,
        ));
        assert!(shadow.owns_css_tree());
        assert_eq!(shadow.shadow_style_index(), None);
    }
    protocol.style_closures.clear();
    let unstyled = descriptor(prepare_render_flags(
        &protocol,
        "shadow-child",
        &indices,
        false,
    ));
    assert!(!unstyled.owns_css_tree());
}

#[test]
fn prepared_shadow_root_push_reuses_reserved_storage_without_owned_payloads() {
    let mut roots = Vec::with_capacity(1);
    let allocation = roots.as_ptr();
    let capacity = roots.capacity();
    for index in 0..1000 {
        WebUIHandler::push_indexed_shadow_style_root(index, &mut roots);
        assert_eq!(roots.as_ptr(), allocation);
        assert_eq!(roots.capacity(), capacity);
        let root = roots
            .pop()
            .unwrap_or_else(|| panic!("prepared root should exist"));
        assert_eq!(root.component_index, index);
        assert!(!root.static_closure_emitted);
        assert_eq!(root.routed_resources.capacity(), 0);
    }
}

#[test]
fn prepared_shadow_descriptors_borrow_the_existing_protocol_records() {
    let protocol = Protocol::new(styled_protocol());
    let resolved = protocol.render_fragments().resolve(protocol.protocol());
    for component in ["light-child", "shadow-child"] {
        let prepared = resolved
            .list_by_id(component)
            .unwrap_or_else(|| panic!("prepared descriptor should exist"));
        let source = &protocol.protocol().fragments[component];
        assert!(std::ptr::eq(
            prepared.fragments.as_ptr(),
            source.fragments.as_ptr()
        ));
        let slot = resolved
            .index(component)
            .unwrap_or_else(|| panic!("prepared slot should exist"));
        let record = resolved
            .list(slot)
            .unwrap_or_else(|| panic!("prepared record should exist"));
        assert_eq!(record.owns_css_tree(), component == "shadow-child");
        if record.owns_css_tree() {
            assert_eq!(
                record.shadow_style_index(),
                protocol.component_index().get(component).copied()
            );
        }
    }
}

#[test]
fn prepared_shadow_roots_preserve_repeated_tree_local_light_css_and_nonce() {
    let protocol = Protocol::new(styled_protocol());
    let mut writer = StyleWriter::with_capacity(4096);
    WebUIHandler::new()
        .render(
            &protocol,
            &test_json!({}),
            &RenderOptions::new("index.html", "/").with_nonce("prepared-nonce"),
            &mut writer,
        )
        .unwrap_or_else(|error| panic!("prepared styled render failed: {error}"));
    assert_eq!(writer.output.matches(".shadow{color:blue}").count(), 2);
    assert_eq!(writer.output.matches(".light{color:red}").count(), 3);
    assert_eq!(writer.output.matches("shadowrootmode=\"open\"").count(), 2);
    assert_eq!(writer.output.matches("nonce=\"prepared-nonce\"").count(), 5);
}

#[test]
fn prepared_shadow_roots_preserve_load_time_metadata_rejections() {
    for defect in ["closure", "hook", "duplicate"] {
        let mut wire = styled_protocol();
        let expected = match defect {
            "closure" => {
                wire.style_closures.remove("shadow-child");
                "component style closure metadata is missing Shadow root"
            }
            "hook" => {
                wire.fragments
                    .get_mut("shadow-child")
                    .unwrap_or_else(|| panic!("shadow record should exist"))
                    .fragments
                    .retain(|fragment| !matches!(fragment.fragment, Some(Fragment::Signal(_))));
                "is missing its compiler style insertion hook"
            }
            _ => {
                wire.style_closures
                    .get_mut("shadow-child")
                    .unwrap_or_else(|| panic!("shadow closure should exist"))
                    .component_tags
                    .push("shadow-child".to_owned());
                "contains duplicate resource"
            }
        };
        let bytes = wire
            .to_protobuf()
            .unwrap_or_else(|error| panic!("encode failed: {error}"));
        let error = match Protocol::from_protobuf(&bytes) {
            Ok(_) => panic!("malformed metadata should not load"),
            Err(error) => error,
        };
        assert!(error.to_string().contains(expected));
        let protocol = Protocol::new(wire);
        let mut writer = StyleWriter::with_capacity(4096);
        let error = WebUIHandler::new()
            .render(
                &protocol,
                &test_json!({}),
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap_err();
        assert!(error.to_string().contains(expected));
        assert!(writer.output.is_empty());
    }
}
