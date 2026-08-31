// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;

fn versioned_plugins() -> [Box<dyn ParserPlugin>; 2] {
    [
        Box::new(crate::plugin::fast_v2::FastV2ParserPlugin::new()),
        Box::new(crate::plugin::fast_v3::FastV3ParserPlugin::new()),
    ]
}

#[cfg(feature = "fs")]
fn source_transform() -> Option<crate::plugin::ComponentSourceTransform> {
    crate::plugin::fast_v2::FastV2ParserPlugin::new()
        .component_processing()
        .source_transform
}

#[test]
#[cfg(feature = "fs")]
fn discovery_registers_generated_filename() {
    let mut fs = webui_test_utils::TestFileSystem::new();
    let html = fs.add_file(
        "components/button.template.html",
        r#"<f-template name="custom-button" shadowrootmode="open"><template @click="{clickHandler($e)}"><slot></slot></template></f-template>"#,
    );
    let mut registry = ComponentRegistry::new();
    registry.set_component_source_transform(source_transform());
    registry
        .register_from_paths(&[html.parent().expect("dir")])
        .expect("discover");
    assert!(registry.contains("custom-button"));
    assert!(!registry.contains("button.template"));
}

#[test]
#[cfg(feature = "fs")]
fn discovery_ignores_generated_filename_without_transform() {
    let mut fs = webui_test_utils::TestFileSystem::new();
    let html = fs.add_file(
        "components/button.template.html",
        r#"<f-template name="custom-button"><template><slot></slot></template></f-template>"#,
    );
    let mut registry = ComponentRegistry::new();
    registry
        .register_from_paths(&[html.parent().expect("dir")])
        .expect("discover");
    assert!(!registry.contains("custom-button"));
    assert!(registry.get_all().next().is_none());
}

#[test]
#[cfg(feature = "fs")]
fn discovery_ignores_unclaimed_non_component_file() {
    let mut fs = webui_test_utils::TestFileSystem::new();
    let html = fs.add_file("components/partial.template.html", "<p>plain fragment</p>");
    let mut registry = ComponentRegistry::new();
    registry.set_component_source_transform(source_transform());
    registry
        .register_from_paths(&[html.parent().expect("dir")])
        .expect("discover");
    assert!(registry.get_all().next().is_none());
}

#[test]
#[cfg(feature = "fs")]
fn discovery_registers_plain_hyphenated_component() {
    let mut fs = webui_test_utils::TestFileSystem::new();
    let html = fs.add_file(
        "components/my-card.html",
        "<template><slot></slot></template>",
    );
    let mut registry = ComponentRegistry::new();
    registry.set_component_source_transform(source_transform());
    registry
        .register_from_paths(&[html.parent().expect("dir")])
        .expect("discover");
    assert!(registry.contains("my-card"));
}

#[test]
#[cfg(feature = "fs")]
fn discovery_rejects_invalid_authored_name() {
    let mut fs = webui_test_utils::TestFileSystem::new();
    let html = fs.add_file(
        "components/widget.template.html",
        r#"<f-template name="notcustom"><template><slot></slot></template></f-template>"#,
    );
    let mut registry = ComponentRegistry::new();
    registry.set_component_source_transform(source_transform());
    let err = registry
        .register_from_paths(&[html.parent().expect("dir")])
        .expect_err("invalid authored name should error");
    assert!(
        matches!(err, ParserError::Component(ref msg) if msg.contains("must contain a hyphen")),
        "unexpected error: {err:?}"
    );
}

#[test]
#[cfg(feature = "fs")]
fn discovery_rejects_authored_name_collision() {
    let mut fs = webui_test_utils::TestFileSystem::new();
    let first = fs.add_file(
        "a/button.template.html",
        r#"<f-template name="custom-button"><template><slot></slot></template></f-template>"#,
    );
    let second = fs.add_file(
        "b/toggle-button.template.html",
        r#"<f-template name="custom-button"><template><slot></slot></template></f-template>"#,
    );
    let mut registry = ComponentRegistry::new();
    registry.set_component_source_transform(source_transform());
    let err = registry
        .register_from_paths(&[first.parent().expect("dir"), second.parent().expect("dir")])
        .expect_err("colliding authored names should error");
    assert!(
        matches!(err, ParserError::Component(ref msg) if msg.contains("already registered")),
        "unexpected error: {err:?}"
    );
}

#[test]
fn webui_plugin_leaves_fast_source_inert() {
    let mut parser = HtmlParser::with_plugin(Box::new(plugin::webui::WebUIParserPlugin::new()));
    parser
        .component_registry
        .register_component(ComponentRegistration::new(
            "file-card",
            r#"<f-template name="named-card"><template><span>{{label}}</span></template></f-template>"#,
            None,
            true,
        ))
        .expect("register component");

    assert!(parser.component_registry.contains("file-card"));
    assert!(!parser.component_registry.contains("named-card"));
    assert_eq!(
        parser
            .component_registry
            .get("file-card")
            .map(|component| component.html_content.as_str()),
        Some(
            r#"<f-template name="named-card"><template><span>{{label}}</span></template></f-template>"#
        )
    );
    assert_eq!(
        parser
            .component_registry
            .component_artifact_source("file-card"),
        None
    );
}

#[test]
fn default_parser_leaves_fast_source_inert() {
    let mut parser = HtmlParser::new();
    parser
        .component_registry
        .register_component(ComponentRegistration::new(
            "file-card",
            r#"<f-template name="named-card"><template><span>{{label}}</span></template></f-template>"#,
            None,
            true,
        ))
        .expect("register component");

    assert!(parser.component_registry.contains("file-card"));
    assert!(!parser.component_registry.contains("named-card"));
    assert_eq!(
        parser
            .component_registry
            .component_artifact_source("file-card"),
        None
    );
}

#[test]
fn plugins_receive_authored_shadow_metadata() {
    for plugin in versioned_plugins() {
        let mut parser = HtmlParser::with_plugin(plugin);
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "x-card",
                "<template shadowrootmode=\"open\"><slot></slot></template>",
                None,
                true,
            ))
            .expect("register component");
        parser
            .parse("index.html", "<x-card>projected</x-card>")
            .expect("parse component");

        let ParserPluginArtifacts::ComponentTemplates(templates) =
            parser.take_plugin_artifacts().expect("plugin artifacts")
        else {
            panic!("expected component templates");
        };
        assert!(templates[0].uses_shadow_dom);
    }
}

#[test]
fn plugins_keep_shadow_css_in_captured_template() {
    for (strategy, expected) in [
        (
            CssStrategy::Link,
            "<link rel=\"stylesheet\" href=\"x-card.css\">",
        ),
        (CssStrategy::Style, "<style>.card{color:red}</style>"),
    ] {
        for plugin in versioned_plugins() {
            let mut parser = HtmlParser::with_plugin_options(
                plugin,
                ParserOptions {
                    css_strategy: strategy,
                    ..ParserOptions::default()
                },
            );
            parser
                .component_registry_mut()
                .register_component(ComponentRegistration::new(
                    "x-card",
                    "<template shadowrootmode=\"open\"><div class=\"card\"></div></template>",
                    Some(".card{color:red}"),
                    true,
                ))
                .expect("register component");
            parser
                .parse("index.html", "<x-card></x-card>")
                .expect("parse component");

            let ParserPluginArtifacts::ComponentTemplates(templates) =
                parser.take_plugin_artifacts().expect("plugin artifacts")
            else {
                panic!("expected component templates");
            };
            assert!(
                templates[0].template.contains(expected),
                "{strategy:?} plugin template missing {expected}: {}",
                templates[0].template
            );
        }
    }
}

#[test]
fn plugin_rejects_effective_light_dom() {
    let mut parser = HtmlParser::with_plugin_options(
        Box::new(crate::plugin::fast_v3::FastV3ParserPlugin::new()),
        ParserOptions {
            css_strategy: CssStrategy::Style,
            dom_strategy: DomStrategy::Light,
            ..ParserOptions::default()
        },
    );
    parser
        .component_registry_mut()
        .register_component(ComponentRegistration::new(
            "x-light",
            "<div class=\"card\"></div>",
            Some(".card{color:red}"),
            true,
        ))
        .expect("register component");
    let error = parser
        .parse("index.html", "<x-light></x-light>")
        .expect_err("FAST must reject Light DOM");
    assert!(matches!(
        error,
        ParserError::Template(ref diagnostic)
            if diagnostic.error_code() == Some(codes::FAST_LIGHT_DOM_UNSUPPORTED)
                && diagnostic.component_name() == Some("x-light")
                && diagnostic.help_text().is_some()
    ));
}

#[test]
fn plugin_build_leaves_ssr_output_style_free() {
    let mut parser = HtmlParser::with_plugin_options(
        Box::new(crate::plugin::fast_v3::FastV3ParserPlugin::new()),
        ParserOptions {
            css_strategy: CssStrategy::Style,
            ..ParserOptions::default()
        },
    );
    parser
        .component_registry_mut()
        .register_component(ComponentRegistration::new(
            "x-card",
            "<template shadowrootmode=\"open\"><div class=\"card\"></div></template>",
            Some(".card{color:red}"),
            true,
        ))
        .expect("register component");
    parser
        .parse("index.html", "<x-card></x-card>")
        .expect("parse component");

    let records = parser.into_fragment_records();
    let mut ssr = String::new();
    for fragment in &records["x-card"].fragments {
        if let Some(web_ui_fragment::Fragment::Raw(ref value)) = fragment.fragment {
            ssr.push_str(&value.value);
        }
    }
    assert!(!ssr.contains("<style>"));
}

fn assert_component_source(plugin: Box<dyn ParserPlugin>) {
    let mut parser =
        HtmlParser::with_plugin_options(plugin, (CssStrategy::Style, DomStrategy::Shadow));
    parser
        .component_registry
        .register_component(ComponentRegistration::new(
            "file-card",
            r#"<f-template name="named-card"><template><f-when value="{{visible}}"><f-repeat value="{{item in items}}"><button @click="{save()}" :config="{config}" ?disabled="{{disabled}}" f-ref="{button}" title="{{title}}">{{item.label}}</button></f-repeat></f-when></template></f-template>"#,
            Some(".root { color: red; }"),
            true,
        ))
        .expect("register component");

    parser
        .parse("index.html", "<named-card></named-card>")
        .expect("parse entry");
    let records = parser.fragment_records.clone();
    assert_stream!(
        records,
        "named-card",
        [
            raw("<template shadowrootmode=\"open\">"),
            structural_matcher("shadow_styles:named-card"),
            if_cond("if-1"),
            raw("</template>"),
        ]
    );
    assert_stream!(records, "if-1", [for_loop("item", "items", "for-1"),]);
    let for_fragments = &records["for-1"].fragments;
    assert!(for_fragments.iter().any(|fragment| {
        matches!(
            fragment.fragment.as_ref(),
            Some(Fragment::Plugin(data)) if data.data == 5u32.to_le_bytes()
        )
    }));
    assert!(!for_fragments.iter().any(|fragment| {
        matches!(
            fragment.fragment.as_ref(),
            Some(Fragment::Raw(raw))
                if raw.value.contains("@click")
                    || raw.value.contains(":config")
                    || raw.value.contains("f-ref")
        )
    }));

    let artifacts = parser.take_plugin_artifacts().expect("artifacts");
    let ParserPluginArtifacts::ComponentTemplates(templates) = artifacts else {
        panic!("expected component template artifacts");
    };
    assert_eq!(templates.len(), 1);
    assert_eq!(templates[0].tag_name, "named-card");
    let template = &templates[0].template;
    assert!(template.contains(r#"<f-template name="named-card" shadowrootmode="open">"#));
    assert!(template.contains("<style>.root { color: red; }</style>"));
    assert!(template.contains(r#"<f-when value="{{visible}}">"#));
    assert!(template.contains(r#"<f-repeat value="{{item in items}}">"#));
    assert!(template.contains(r#"@click="{save()}""#));
    assert!(template.contains(r#":config="{config}""#));
    assert!(template.contains(r#"?disabled="{{disabled}}""#));
    assert!(template.contains(r#"f-ref="{button}""#));
    assert!(template.contains(r#"title="{{title}}""#));
    assert!(!template.contains("file-card"));
}

#[test]
fn v2_uses_authored_component_source() {
    assert_component_source(Box::new(plugin::fast_v2::FastV2ParserPlugin::new()));
}

#[test]
#[allow(deprecated)]
fn compatibility_plugin_uses_authored_component_source() {
    assert_component_source(Box::new(plugin::fast::FastParserPlugin::new()));
}

#[test]
fn v3_uses_authored_component_source() {
    assert_component_source(Box::new(plugin::fast_v3::FastV3ParserPlugin::new()));
}

#[test]
fn v3_style_strategy_trails_style_in_ssr_and_artifact() {
    let css = ":host { display: inline-flex; } \
@media (forced-colors: active) { :host { color: CanvasText; } }";
    let style_block = format!("<style>{css}</style>");
    let mut parser = HtmlParser::with_plugin_options(
        Box::new(plugin::fast_v3::FastV3ParserPlugin::new()),
        (CssStrategy::Style, DomStrategy::Shadow),
    );
    parser
        .component_registry
        .register_component(ComponentRegistration::new(
            "custom-btn",
            r#"<f-template name="custom-btn"><template @click="{click($e)}"><slot name="start" f-ref="{start}"></slot><span class="content"><slot f-slotted="{slotted}"></slot></span></template></f-template>"#,
            Some(css),
            true,
        ))
        .expect("register component");
    parser
        .parse("index.html", "<custom-btn></custom-btn>")
        .expect("parse entry");

    let records = parser.fragment_records.clone();
    let ssr_raw: String = records["custom-btn"]
        .fragments
        .iter()
        .filter_map(|fragment| match fragment.fragment.as_ref() {
            Some(Fragment::Raw(raw)) => Some(raw.value.as_str()),
            _ => None,
        })
        .collect();
    assert!(!ssr_raw.contains("<style>"));
    assert!(!ssr_raw.contains("{{styles}}"));

    let ParserPluginArtifacts::ComponentTemplates(templates) =
        parser.take_plugin_artifacts().expect("artifacts")
    else {
        panic!("expected component template artifacts");
    };
    let template = &templates[0].template;
    assert!(template.contains(&style_block));
    let style_at = template.find("<style>").expect("artifact style");
    for binding in ["@click=", "f-ref=", "f-slotted="] {
        assert!(template.find(binding).expect(binding) < style_at);
    }
    assert!(template.contains("</style></template>"));
    assert!(!template.contains("{{styles}}"));
}

fn binding_counts(css_strategy: CssStrategy) -> Vec<u32> {
    let mut parser = HtmlParser::with_plugin_options(
        Box::new(plugin::fast_v3::FastV3ParserPlugin::new()),
        (css_strategy, DomStrategy::Shadow),
    );
    parser
        .component_registry
        .register_component(ComponentRegistration::new(
            "custom-btn",
            r#"<f-template name="custom-btn"><template @click="{click($e)}"><slot name="start" f-ref="{start}"></slot><span class="content"><slot f-slotted="{slotted}"></slot></span></template></f-template>"#,
            Some("@media screen { :host { color: red; } } .content { display: inherit; }"),
            true,
        ))
        .expect("register component");
    parser
        .parse("index.html", "<custom-btn></custom-btn>")
        .expect("parse entry");
    parser.fragment_records["custom-btn"]
        .fragments
        .iter()
        .filter_map(|fragment| {
            let Some(Fragment::Plugin(data)) = fragment.fragment.as_ref() else {
                return None;
            };
            (data.data.len() == 4)
                .then(|| u32::from_le_bytes(data.data[..4].try_into().expect("four-byte count")))
        })
        .collect()
}

#[test]
fn v3_style_and_module_emit_equal_binding_counts() {
    let style_counts = binding_counts(CssStrategy::Style);
    assert!(!style_counts.is_empty());
    assert_eq!(style_counts, binding_counts(CssStrategy::Module));
}

#[test]
fn client_attributes_are_counted_in_source_order_without_markers() {
    let mut parser = HtmlParser::with_plugin(Box::new(plugin::fast_v3::FastV3ParserPlugin::new()));
    parser
        .component_registry
        .register_component(ComponentRegistration::new(
            "binding-card",
            r#"<f-template name="binding-card"><template><div @click="{save()}" :config="{config}" f-ref="{root}" f-slotted="{slot}" f-children="{children}" title="{{title}}"><span @focus="{focus()}" :value="{value}">{{label}}</span></div></template></f-template>"#,
            None,
            true,
        ))
        .expect("register component");

    parser
        .parse("index.html", "<binding-card></binding-card>")
        .expect("parse entry");
    let fragments = &parser.fragment_records["binding-card"].fragments;
    let counts: Vec<u32> = fragments
        .iter()
        .filter_map(|fragment| {
            let Some(Fragment::Plugin(data)) = fragment.fragment.as_ref() else {
                return None;
            };
            (data.data.len() == 4)
                .then(|| u32::from_le_bytes(data.data[..4].try_into().expect("four-byte count")))
        })
        .collect();
    assert_eq!(counts, vec![6, 2]);

    for fragment in fragments {
        if let Some(Fragment::Raw(raw)) = fragment.fragment.as_ref() {
            for client_attr in [
                "@click",
                ":config",
                "f-ref",
                "f-slotted",
                "f-children",
                "@focus",
                ":value",
                "data-webui-internal-",
            ] {
                assert!(!raw.value.contains(client_attr));
            }
        }
    }

    let ParserPluginArtifacts::ComponentTemplates(templates) =
        parser.take_plugin_artifacts().expect("artifacts")
    else {
        panic!("expected component template artifacts");
    };
    for client_attr in [
        "@click",
        ":config",
        "f-ref",
        "f-slotted",
        "f-children",
        "@focus",
        ":value",
    ] {
        assert!(templates[0].template.contains(client_attr));
    }
    assert!(!templates[0].template.contains("data-webui-internal-"));
}

fn root_template_binding_data(plugin: Box<dyn ParserPlugin>) -> Vec<Vec<u8>> {
    let mut parser = HtmlParser::with_plugin(plugin);
    parser
        .component_registry
        .register_component(ComponentRegistration::new(
            "root-binding-card",
            r#"<f-template name="root-binding-card" shadowrootmode="open"><template @click="{clickHandler($e)}" @keydown="{keydownHandler($e)}" ?focusgroupstart="{{selected}}"><span @focus="{focus()}">{{label}}</span></template></f-template>"#,
            None,
            true,
        ))
        .expect("register component");
    parser
        .parse("index.html", "<root-binding-card></root-binding-card>")
        .expect("parse entry");
    let fragments = &parser.fragment_records["root-binding-card"].fragments;
    let data: Vec<Vec<u8>> = fragments
        .iter()
        .filter_map(|fragment| {
            let Some(Fragment::Plugin(data)) = fragment.fragment.as_ref() else {
                return None;
            };
            Some(data.data.clone())
        })
        .collect();

    let raw: String = fragments
        .iter()
        .filter_map(|fragment| match fragment.fragment.as_ref() {
            Some(Fragment::Raw(raw)) => Some(raw.value.as_str()),
            _ => None,
        })
        .collect();
    for client_attr in ["@click", "@keydown", "?focusgroupstart", "@focus"] {
        assert!(!raw.contains(client_attr));
    }
    assert!(raw.contains(r#"shadowrootmode="open""#));
    data
}

#[test]
fn v2_counts_root_template_bindings() {
    let data = root_template_binding_data(Box::new(plugin::fast_v2::FastV2ParserPlugin::new()));
    let decoded: Vec<_> = data
        .iter()
        .map(|bytes| {
            webui_protocol::FastElementData::decode_v2(bytes)
                .expect("FAST 2 element data should decode")
        })
        .collect();
    assert_eq!(
        decoded,
        vec![
            (webui_protocol::FastElementData { binding_count: 3 }, true),
            (webui_protocol::FastElementData { binding_count: 1 }, false),
        ]
    );
}

#[test]
fn v3_counts_root_template_bindings() {
    let data = root_template_binding_data(Box::new(plugin::fast_v3::FastV3ParserPlugin::new()));
    let counts: Vec<_> = data
        .iter()
        .map(|bytes| {
            webui_protocol::FastElementData::decode(bytes)
                .expect("FAST 3 element data should decode")
                .binding_count
        })
        .collect();
    assert_eq!(counts, vec![3, 1]);
}

#[test]
fn route_only_component_uses_retained_authored_source() {
    let mut parser = HtmlParser::with_plugin(Box::new(plugin::fast_v2::FastV2ParserPlugin::new()));
    parser
        .component_registry
        .register_component(ComponentRegistration::new(
            "route-card",
            r#"<f-template name="route-card"><template><f-when value="{{visible}}"><span>{{label}}</span></f-when></template></f-template>"#,
            None,
            true,
        ))
        .expect("register component");

    parser
        .parse(
            "index.html",
            r#"<route path="/card" component="route-card" exact />"#,
        )
        .expect("parse route");
    assert!(parser.fragment_records.contains_key("route-card"));

    let ParserPluginArtifacts::ComponentTemplates(templates) =
        parser.take_plugin_artifacts().expect("artifacts")
    else {
        panic!("expected component template artifacts");
    };
    assert_eq!(templates.len(), 1);
    assert!(templates[0]
        .template
        .contains(r#"<f-when value="{{visible}}">"#));
}

#[test]
fn v2_converts_double_quoted_when_condition() {
    let mut parser = HtmlParser::with_plugin(Box::new(plugin::fast_v2::FastV2ParserPlugin::new()));
    parser
        .component_registry
        .register_component(ComponentRegistration::new(
            "status-card",
            r#"<f-template name="status-card"><template><f-when value='{{status == "ready"}}'><span>{{label}}</span></f-when></template></f-template>"#,
            None,
            true,
        ))
        .expect("register component");

    let parser_content = parser
        .component_registry
        .get("status-card")
        .map(|component| component.html_content.clone())
        .expect("registered component");
    assert_eq!(
        parser_content,
        r#"<template><if condition='status == "ready"'><span>{{label}}</span></if></template>"#
    );

    let mut ssr = HtmlParser::new();
    ssr.parse("status-card.html", &parser_content)
        .expect("converted parser content parses cleanly");
}
