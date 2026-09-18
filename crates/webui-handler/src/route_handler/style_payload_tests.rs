// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;
use webui_protocol::{
    ComponentData, ComponentStyleClosure, FragmentList, StyleChunk, WebUIFragment,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

mod bootstrap;

fn protocol(strategy: CssStrategy) -> WebUIProtocol {
    let mut protocol = WebUIProtocol::default();
    protocol.set_css_strategy(strategy);
    for (tag, css, href) in [("z-card", ".z{}", "/z.css"), ("a-card", ".a{}", "/a.css")] {
        protocol.components.insert(
            tag.to_owned(),
            ComponentData {
                css: css.to_owned(),
                css_href: href.to_owned(),
                template_json: r#"{"h":""}"#.to_owned(),
                ..Default::default()
            },
        );
        protocol
            .fragments
            .insert(tag.to_owned(), FragmentList::default());
    }
    for (root, members) in [
        ("z-card", vec!["z-card", "a-card"]),
        ("index.html", vec!["z-card", "a-card"]),
        ("empty-root", vec![]),
        ("a-card", vec!["a-card"]),
    ] {
        protocol.fragments.entry(root.to_owned()).or_default();
        protocol.style_closures.insert(
            root.to_owned(),
            ComponentStyleClosure {
                component_tags: members.into_iter().map(str::to_owned).collect(),
                style_chunks: Vec::new(),
            },
        );
    }
    protocol
}

fn resources(strategy: CssStrategy) -> &'static str {
    match strategy {
        CssStrategy::Link => {
            r#"{"a-card":{"href":"/a.css","kind":"link"},"z-card":{"href":"/z.css","kind":"link"}}"#
        }
        CssStrategy::Style => {
            r#"{"a-card":{"css":".a{}","kind":"style"},"z-card":{"css":".z{}","kind":"style"}}"#
        }
        CssStrategy::Module => {
            r#"{"a-card":{"css":".a{}","kind":"module","specifier":"a-card"},"z-card":{"css":".z{}","kind":"module","specifier":"z-card"}}"#
        }
    }
}

fn expected(strategy: CssStrategy, closures: &str, resources: &str) -> String {
    format!(
        r#"{{"closures":{closures},"resources":{resources},"strategy":"{}","version":1}}"#,
        strategy.wire_name()
    )
}

fn full_payload<'a>(
    protocol: &'a WebUIProtocol,
    roots: impl IntoIterator<Item = &'a str>,
    chunks: &HashMap<&str, u32>,
) -> Result<BorrowedComponentStyleDelta<'a>, HandlerError> {
    collect_borrowed_component_style_delta(protocol, roots, &[], &HashMap::new(), chunks)
}

fn assert_payload(
    protocol: &WebUIProtocol,
    roots: &[&str],
    closures: &str,
    resources: &str,
) -> TestResult {
    let exact = expected(protocol.css_strategy(), closures, resources);
    let chunks = protocol.style_chunk_index();
    let borrowed = full_payload(protocol, roots.iter().copied(), &chunks)?;
    assert_eq!(serde_json::to_string(&borrowed)?, exact);
    let owned = collect_component_styles(protocol, roots.iter().copied())?;
    assert_eq!(serde_json::to_string(&owned)?, exact);
    let index = build_style_resource_index(protocol);
    let delta = collect_borrowed_component_style_delta(
        protocol,
        roots.iter().copied(),
        &[],
        &index,
        &chunks,
    )?;
    assert_eq!(serde_json::to_string(&delta)?, exact);
    Ok(())
}

#[test]
fn exact_payload_bytes_for_empty_subset_all_and_duplicate_roots() -> TestResult {
    for strategy in [CssStrategy::Link, CssStrategy::Style, CssStrategy::Module] {
        let data = protocol(strategy);
        assert_payload(&data, &[], "{}", "{}")?;
        assert_payload(&data, &["absent"], "{}", "{}")?;
        assert_payload(&data, &["empty-root"], r#"{"empty-root":[]}"#, "{}")?;
        assert_payload(
            &data,
            &["z-card"],
            r#"{"z-card":["z-card","a-card"]}"#,
            resources(strategy),
        )?;
        assert_payload(
            &data,
            &[
                "z-card",
                "empty-root",
                "index.html",
                "a-card",
                "z-card",
                "absent",
            ],
            r#"{"a-card":["a-card"],"empty-root":[],"index.html":["z-card","a-card"],"z-card":["z-card","a-card"]}"#,
            resources(strategy),
        )?;
        let mut empty = WebUIProtocol::default();
        empty.set_css_strategy(strategy);
        assert_payload(&empty, &["absent"], "{}", "{}")?;
    }
    Ok(())
}

#[test]
fn payload_borrows_protocol_storage_with_canonical_root_inputs() -> TestResult {
    let data = protocol(CssStrategy::Style);
    let selected = {
        let root = String::from("z-card");
        let (canonical, _) = data
            .style_closures
            .get_key_value(root.as_str())
            .ok_or("root")?;
        let chunks = data.style_chunk_index();
        full_payload(&data, [canonical.as_str()], &chunks)?
    };
    let (name, _) = data.style_closures.get_key_value("z-card").ok_or("root")?;
    assert!(std::ptr::eq(selected.closures[0].root, name.as_str()));
    for resource in &selected.resources {
        let component = data.components.get(resource.name).ok_or("component")?;
        assert!(std::ptr::eq(resource.resource, component.css.as_str()));
    }
    assert_eq!(
        serde_json::to_string(&selected)?,
        expected(
            CssStrategy::Style,
            r#"{"z-card":["z-card","a-card"]}"#,
            resources(CssStrategy::Style),
        ),
    );
    let request_root = String::from("z-card");
    let request_payload = full_payload(&data, [request_root.as_str()], &HashMap::new())?;
    assert!(std::ptr::eq(
        request_payload.closures[0].root,
        request_root.as_str(),
    ));
    assert_eq!(
        serde_json::to_string(&request_payload)?,
        serde_json::to_string(&selected)?,
    );
    Ok(())
}

#[test]
fn duplicate_members_keep_first_discovery_order() -> TestResult {
    let mut data = protocol(CssStrategy::Style);
    data.style_closures
        .get_mut("z-card")
        .ok_or("root")?
        .component_tags
        .extend(["z-card".to_owned(), "a-card".to_owned()]);
    assert_payload(
        &data,
        &["z-card"],
        r#"{"z-card":["z-card","a-card"]}"#,
        resources(CssStrategy::Style),
    )
}

#[test]
fn shared_light_styles_keep_shadow_ownership_closure_order() -> TestResult {
    let mut data = protocol(CssStrategy::Style);
    data.components
        .get_mut("z-card")
        .ok_or("shadow root")?
        .uses_shadow_dom = true;
    data.fragments
        .get_mut("z-card")
        .ok_or("shadow body")?
        .fragments = vec![WebUIFragment::component("a-card")];
    data.fragments
        .get_mut("index.html")
        .ok_or("entry")?
        .fragments = vec![
        WebUIFragment::component("z-card"),
        WebUIFragment::component("a-card"),
    ];
    data.populate_style_closures(&["index.html"]);
    assert_payload(
        &data,
        &["index.html", "z-card", "a-card"],
        r#"{"a-card":["a-card"],"index.html":["a-card"],"z-card":["z-card","a-card"]}"#,
        resources(CssStrategy::Style),
    )
}

#[test]
fn sparse_selection_preserves_existing_reservations_and_selected_members() -> TestResult {
    let mut data = protocol(CssStrategy::Style);
    for index in 0..4096 {
        data.components
            .insert(format!("unselected-{index}"), ComponentData::default());
    }
    let covered = covered_components(&data, &["z-card"], None);
    assert!(covered.is_empty());
    assert_eq!(
        covered.capacity(),
        HashSet::<&str>::with_capacity(data.components.len()).capacity(),
    );
    let selected = full_payload(&data, ["z-card"], &HashMap::new())?;
    assert_eq!(selected.resources.len(), 2);
    assert_eq!(selected.closures.len(), 1);
    assert_eq!(selected.ordered.len(), 2);
    assert!(selected.resources.capacity() < 16);
    assert_eq!(
        serde_json::to_string(&selected)?,
        expected(
            CssStrategy::Style,
            r#"{"z-card":["z-card","a-card"]}"#,
            resources(CssStrategy::Style),
        ),
    );
    Ok(())
}

#[test]
fn small_selections_preserve_existing_borrowed_vector_reservations() -> TestResult {
    let data = protocol(CssStrategy::Style);
    let chunks = data.style_chunk_index();
    let index = build_style_resource_index(&data);
    for (root, units) in [("empty-root", 0), ("a-card", 1), ("z-card", 2)] {
        let complete = full_payload(&data, [root], &chunks)?;
        let delta = collect_borrowed_component_style_delta(&data, [root], &[], &index, &chunks)?;
        for selected in [&complete, &delta] {
            assert_eq!(selected.resources.len(), units);
            assert_eq!(selected.resources.capacity(), units);
            assert_eq!(selected.ordered.len(), units);
            assert_eq!(selected.ordered.capacity(), units);
            assert_eq!(selected.closures.len(), 1);
            assert_eq!(selected.closures.capacity(), 1);
        }
    }
    Ok(())
}

#[test]
fn chunk_selection_preserves_members_coverage_and_inventory_kinds() -> TestResult {
    for strategy in [CssStrategy::Link, CssStrategy::Style, CssStrategy::Module] {
        let mut data = protocol(strategy);
        data.style_closures
            .get_mut("index.html")
            .ok_or("entry")?
            .style_chunks = vec![0];
        data.style_chunks.push(StyleChunk {
            name: "shared".to_owned(),
            css: ".z{}.a{}".to_owned(),
            css_href: "/shared.css".to_owned(),
            component_tags: vec!["z-card".to_owned(), "a-card".to_owned()],
        });
        let resource = match strategy {
            CssStrategy::Link => {
                r#"{"shared":{"href":"/shared.css","kind":"link","members":["z-card","a-card"]}}"#
            }
            CssStrategy::Style => {
                r#"{"shared":{"css":".z{}.a{}","kind":"style","members":["z-card","a-card"]}}"#
            }
            CssStrategy::Module => {
                r#"{"shared":{"css":".z{}.a{}","kind":"module","members":["z-card","a-card"],"specifier":"shared"}}"#
            }
        };
        assert_payload(
            &data,
            &["a-card", "index.html", "z-card"],
            r#"{"index.html":["shared"]}"#,
            resource,
        )?;
        assert_payload(&data, &["z-card"], r#"{"z-card":["shared"]}"#, resource)?;
        let component_index = build_component_index(&data);
        let component_inventory = vec![u8::MAX; component_index.len().div_ceil(8)];
        let owned = collect_component_styles_for_inventory(
            &data,
            ["index.html"],
            &component_inventory,
            &component_index,
        )?;
        assert_eq!(
            serde_json::to_string(&owned)?,
            expected(strategy, r#"{"index.html":["shared"]}"#, resource),
        );
        let style_index = build_style_resource_index(&data);
        let mut inventory = vec![0; style_index.len().div_ceil(8)];
        set_component(
            &mut inventory,
            *style_index.get("shared").ok_or("chunk index")?,
        );
        let delta = collect_borrowed_component_style_delta(
            &data,
            ["a-card", "index.html", "z-card"],
            &inventory,
            &style_index,
            &data.style_chunk_index(),
        )?;
        assert_eq!(
            serde_json::to_string(&delta)?,
            expected(strategy, r#"{"index.html":["shared"]}"#, "{}"),
        );
    }
    Ok(())
}

#[test]
fn singleton_chunks_omit_members_and_partial_members_do_not_claim_chunks() -> TestResult {
    let mut data = protocol(CssStrategy::Style);
    data.style_chunks.push(StyleChunk {
        name: "single".to_owned(),
        css: ".z{}".to_owned(),
        css_href: String::new(),
        component_tags: vec!["z-card".to_owned()],
    });
    assert_payload(
        &data,
        &["z-card"],
        r#"{"z-card":["single","a-card"]}"#,
        r#"{"a-card":{"css":".a{}","kind":"style"},"single":{"css":".z{}","kind":"style"}}"#,
    )?;
    data.style_chunks[0]
        .component_tags
        .push("a-card".to_owned());
    assert_payload(
        &data,
        &["a-card"],
        r#"{"a-card":["a-card"]}"#,
        r#"{"a-card":{"css":".a{}","kind":"style"}}"#,
    )
}

#[test]
fn owned_component_and_route_apis_share_shape_and_inventory_filters() -> TestResult {
    let mut data = protocol(CssStrategy::Style);
    data.fragments
        .get_mut("index.html")
        .ok_or("entry")?
        .fragments = vec![WebUIFragment::route("/selected", "z-card")];
    let runtime = Protocol::new(data);
    let exact = expected(
        CssStrategy::Style,
        r#"{"z-card":["z-card","a-card"]}"#,
        resources(CssStrategy::Style),
    );
    let components = runtime.render_component_templates(&["z-card", "z-card"], "")?;
    assert_eq!(
        serde_json::to_string(&components["componentStyles"])?,
        exact
    );
    for partial in [
        runtime.render_partial(Value::Null, "index.html", "/selected", "")?,
        runtime.render_partial_json("null", "index.html", "/selected", "")?,
    ] {
        let parsed: Value = serde_json::from_str(&partial)?;
        assert_eq!(serde_json::to_string(&parsed["componentStyles"])?, exact);
        assert!(partial.contains(&format!(r#""componentStyles":{exact}"#)));
        assert_eq!(parsed["state"], Value::Null);
    }
    let shared = runtime.render_component_templates(&["a-card"], "")?;
    let inventory = shared["inventory"].as_str().ok_or("inventory")?;
    let subset = runtime.render_component_templates(&["z-card"], inventory)?;
    assert_eq!(
        serde_json::to_string(&subset["componentStyles"])?,
        expected(
            CssStrategy::Style,
            r#"{"z-card":["z-card","a-card"]}"#,
            r#"{"z-card":{"css":".z{}","kind":"style"}}"#,
        ),
    );
    let full = runtime.render_component_templates(
        &["z-card"],
        components["inventory"].as_str().ok_or("inventory")?,
    )?;
    assert_eq!(
        serde_json::to_string(&full["componentStyles"])?,
        expected(CssStrategy::Style, "{}", "{}"),
    );
    Ok(())
}

#[test]
fn missing_metadata_and_resources_fail_during_selection() -> TestResult {
    let mut data = protocol(CssStrategy::Style);
    data.style_closures.remove("z-card");
    assert!(matches!(
        full_payload(&data, ["z-card"], &HashMap::new()),
        Err(HandlerError::Invariant(message)) if message == "component style closure metadata is missing root `z-card`"
    ));
    data.components.remove("a-card");
    assert!(matches!(
        full_payload(&data, ["index.html"], &HashMap::new()),
        Err(HandlerError::Invariant(message)) if message == "component style closure `index.html` references missing resource `a-card`"
    ));
    data.style_closures.clear();
    assert!(matches!(
        full_payload(&data, [], &HashMap::new()),
        Err(HandlerError::Invariant(message)) if message == "component style closure metadata is required by this protocol"
    ));
    Ok(())
}

#[test]
fn custom_style_serializers_propagate_writer_errors() -> TestResult {
    struct Reject;
    impl std::io::Write for Reject {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("style transport failure"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let data = protocol(CssStrategy::Module);
    let selected = full_payload(&data, ["z-card"], &HashMap::new())?;
    let error = serde_json::to_writer(Reject, &selected)
        .err()
        .ok_or("writer succeeded")?;
    assert!(error.is_io());
    assert_eq!(error.to_string(), "style transport failure");
    Ok(())
}
