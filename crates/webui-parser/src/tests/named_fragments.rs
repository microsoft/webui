// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use crate::named_fragments::record_id;
use crate::plugin::{webui::WebUIParserPlugin, ParserPluginArtifacts};
use crate::*;
use webui_protocol::{web_ui_fragment::Fragment, BoundaryPhase, WebUIProtocol};

#[test]
fn directive_prefixes_do_not_initialize_fragment_or_module_tracking() {
    let mut parser = HtmlParser::new();
    parser
        .parse(
            "index.html",
            "<fragmentation>one</fragmentation><renderer>two</renderer><scripture>three</scripture>",
        )
        .expect("ordinary elements");
    assert!(!parser.has_named_fragments);
    assert!(parser.module_entry_sites.is_none());
}

#[test]
fn source_features_cover_case_and_tag_boundaries() {
    for source in [
        "<fragment>",
        "<fragment/>",
        "<FRAGMENT name='x'>",
        "<render\t",
        "<render\n",
    ] {
        assert!(named_fragments::contains_directives(source), "{source}");
        assert!(
            named_fragments::SourceFeatures::scan(source).directives,
            "{source}"
        );
    }
    for source in ["<fragmentation>", "<renderer>", "<render-x>", "<scripture>"] {
        assert!(!named_fragments::contains_directives(source), "{source}");
        let features = named_fragments::SourceFeatures::scan(source);
        assert!(!features.directives && !features.scripts, "{source}");
    }
    let features =
        named_fragments::SourceFeatures::scan("<SCRIPT type='module'></SCRIPT><render/>");
    assert!(features.directives && features.scripts);
}

#[test]
fn local_declarations_compile_once_and_resolve_forward_mutual_calls() {
    let mut parser = HtmlParser::new();
    parser.parse("index.html", r#"<body>
        <render fragment="a" scope="{{treeData}}" as="items"/>
        <fragment name="a"><for each="item in items"><span>{{item.name}}</span><if condition="item.children.length"><render fragment="b" scope="item.children" as="nodes"/></if></for></fragment>
        <fragment name="b"><render fragment="a" scope="nodes" as="items"/></fragment>
    </body>"#).expect("parse recursive graph");
    let records = parser.into_fragment_records();
    assert!(records.contains_key(&record_id("index.html", "a")));
    assert!(records.contains_key(&record_id("index.html", "b")));
    assert_eq!(records.len(), 5);
    let calls: Vec<_> = records
        .values()
        .flat_map(|list| &list.fragments)
        .filter_map(|fragment| match &fragment.fragment {
            Some(Fragment::Render(render)) => Some(render),
            _ => None,
        })
        .collect();
    assert_eq!(calls.len(), 3);
    assert!(calls
        .iter()
        .any(|call| call.scope == "treeData" && call.alias == "items"));
    assert!(!records.values().flat_map(|list| &list.fragments).any(|fragment| {
        matches!(&fragment.fragment, Some(Fragment::Raw(raw)) if raw.value.contains("<fragment") || raw.value.contains("<render"))
    }));
    let protocol = WebUIProtocol::new(records);
    WebUIProtocol::from_protobuf(&protocol.to_protobuf().expect("encode"))
        .expect("decode cyclic references");
}

#[test]
fn local_names_are_isolated_by_component_owner() {
    let mut parser = HtmlParser::new();
    for (tag, body) in [("x-first", "first"), ("x-second", "second")] {
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                tag,
                &format!("<fragment name=\"same\">{body}</fragment><render fragment=\"same\"/>"),
                None,
                false,
            ))
            .expect("register");
    }
    parser
        .parse(
            "index.html",
            "<body><x-first></x-first><x-second></x-second></body>",
        )
        .expect("parse");
    let records = parser.into_fragment_records();
    for tag in ["x-first", "x-second"] {
        assert!(records.contains_key(&record_id(tag, "same")));
    }
}

#[test]
fn unused_bodies_are_validated_but_resources_and_tokens_are_pruned() {
    let mut parser = HtmlParser::with_plugin(Box::new(WebUIParserPlugin::new()));
    parser
        .component_registry_mut()
        .register_component(ComponentRegistration::new(
            "x-unused",
            "<strong>{{secret}}</strong>",
            Some("strong{color:var(--unused-component)}"),
            false,
        ))
        .expect("register");
    parser.parse("index.html", r#"<body>
        <fragment name="unused"><style>.unused{color:var(--unused-inline)}</style><script type="module" src="/unused.js"></script><x-unused></x-unused></fragment>
        <fragment name="used"><style>.used{color:var(--used)}</style><span>used</span></fragment>
        <render fragment="used"/>
    </body>"#).expect("parse");
    assert_eq!(parser.take_tokens(), ["used"]);
    assert!(parser.module_entry_srcs().is_empty());
    assert!(!parser.has_fragment("x-unused"));
    assert!(!parser.has_fragment(&record_id("index.html", "unused")));
    let ParserPluginArtifacts::ComponentTemplates(artifacts) =
        parser.take_plugin_artifacts().expect("artifacts")
    else {
        panic!("expected component artifacts");
    };
    assert!(artifacts.is_empty());
}

#[test]
fn unused_invalid_declaration_bodies_do_not_escape_validation() {
    for body in [
        "<if>invalid</if>",
        "<for each=\"bad\">invalid</for>",
        "<render fragment=\"missing\"/>",
        "<fragment name=\"nested\">invalid</fragment>",
        "<style>.broken{</style>",
        "<span>",
    ] {
        let mut parser = HtmlParser::new();
        let source = format!("<body><fragment name=\"unused\">{body}</fragment></body>");
        assert!(parser.parse("index.html", &source).is_err(), "{source}");
    }
}

#[test]
fn fragment_directives_reject_invalid_placements_and_attributes() {
    let cases = [
        (
            "<html><fragment name=\"a\"/><body></body></html>",
            codes::INVALID_FRAGMENT_PLACEMENT,
        ),
        (
            "<body><div><fragment name=\"a\"/></div></body>",
            codes::INVALID_FRAGMENT_PLACEMENT,
        ),
        (
            "<body><if condition=\"ok\"><fragment name=\"a\"/></if></body>",
            codes::INVALID_FRAGMENT_PLACEMENT,
        ),
        (
            "<body><for each=\"x in xs\"><fragment name=\"a\"/></for></body>",
            codes::INVALID_FRAGMENT_PLACEMENT,
        ),
        (
            "<body><fragment name=\"a\"/><fragment name=\"a\"/></body>",
            codes::DUPLICATE_FRAGMENT,
        ),
        (
            "<body><fragment name=\"{{name}}\"/></body>",
            codes::INVALID_FRAGMENT,
        ),
        (
            "<body><fragment name=\"a\" class=\"x\"/></body>",
            codes::INVALID_FRAGMENT_ATTRIBUTE,
        ),
        (
            "<body><fragment name=\"a\" name=\"b\"/></body>",
            codes::INVALID_FRAGMENT_ATTRIBUTE,
        ),
        (
            "<body><render fragment=\"missing\"/></body>",
            codes::UNKNOWN_FRAGMENT,
        ),
        (
            "<body><fragment name=\"a\"/><render fragment=\"a\">text</render></body>",
            codes::INVALID_RENDER,
        ),
        (
            "<body><fragment name=\"a\"/><render fragment=\"a\"><span/></render></body>",
            codes::INVALID_RENDER,
        ),
        (
            "<body><fragment name=\"a\"/><render fragment=\"a\" :scope=\"{{x}}\" as=\"x\"/></body>",
            codes::INVALID_FRAGMENT_ATTRIBUTE,
        ),
        (
            "<body><fragment name=\"a\"/><render fragment=\"a\" scope=\"x\"/></body>",
            codes::INVALID_RENDER_SCOPE,
        ),
        (
            "<body><fragment name=\"a\"/><render fragment=\"a\" as=\"x\"/></body>",
            codes::INVALID_RENDER_SCOPE,
        ),
    ];
    for (source, code) in cases {
        let mut parser = HtmlParser::new();
        let ParserError::Template(diagnostic) =
            parser.parse("index.html", source).expect_err(source)
        else {
            panic!("expected structured authoring error: {source}");
        };
        assert_eq!(
            diagnostic.error_code(),
            Some(code),
            "{source}: {diagnostic}"
        );
        assert!(diagnostic.to_string().contains("help:"));
        assert!(diagnostic.to_string().contains("index.html:1:"));
    }
}

#[test]
fn all_raw_and_inert_contexts_reject_new_directives() {
    for tag in [
        "script",
        "style",
        "textarea",
        "title",
        "xmp",
        "iframe",
        "noembed",
        "noframes",
        "noscript",
        "plaintext",
        "template",
    ] {
        for directive in ["<fragment name=\"a\"/>", "<render fragment=\"a\"/>"] {
            let source = format!("<body><{tag}>{directive}</{tag}></body>");
            let mut parser = HtmlParser::new();
            let error = parser.parse("index.html", &source).expect_err(&source);
            assert!(
                matches!(error, ParserError::Template(diagnostic) if diagnostic.error_code() == Some(codes::INVALID_FRAGMENT_PLACEMENT)),
                "{source}"
            );
        }
    }
}

#[test]
fn render_input_and_alias_grammar_matches_supported_paths() {
    for scope in [
        "items",
        "{{items}}",
        "{{ treeData.children.length }}",
        "_root.field2",
        "tree.length",
    ] {
        let source = format!("<body><fragment name=\"tree-items\"/><render fragment=\"tree-items\" scope=\"{scope}\" as=\"_items2\"/></body>");
        HtmlParser::new()
            .parse("index.html", &source)
            .expect(&source);
    }
    for scope in [
        "",
        "{{}}",
        "{{{items}}}",
        "{{a}} {{b}}",
        "items.0",
        "items[0]",
        "items..child",
        ".items",
        "items.",
        "a + b",
        "data-name",
    ] {
        let source = format!("<body><fragment name=\"a\"/><render fragment=\"a\" scope=\"{scope}\" as=\"items\"/></body>");
        let error = HtmlParser::new()
            .parse("index.html", &source)
            .expect_err(&source);
        assert!(
            matches!(error, ParserError::Template(diagnostic) if diagnostic.error_code() == Some(codes::INVALID_RENDER_SCOPE)),
            "{source}"
        );
    }
    for alias in ["", "a.b", "a-b", "{{a}}", "0name", "$name", " name"] {
        let source = format!("<body><fragment name=\"a\"/><render fragment=\"a\" scope=\"data\" as=\"{alias}\"/></body>");
        assert!(
            HtmlParser::new().parse("index.html", &source).is_err(),
            "{source}"
        );
    }
}

#[test]
fn comments_and_parameterless_empty_calls_emit_no_directive_dom() {
    let mut parser = HtmlParser::new();
    parser
        .parse(
            "index.html",
            r#"<body><!-- <render fragment="missing"/> -->
        <fragment name="empty"/><render fragment="empty"> <!-- ignored --> </render>
    </body>"#,
        )
        .expect("parse");
    let records = parser.into_fragment_records();
    assert!(records[&record_id("index.html", "empty")]
        .fragments
        .is_empty());
    assert!(records["index.html"].fragments.iter().any(|fragment| matches!(
        &fragment.fragment, Some(Fragment::Render(render)) if render.scope.is_empty() && render.alias.is_empty()
    )));
}

#[test]
fn fragment_boundaries_participate_in_repeat_and_nesting_analyses() {
    let cases = [
        (
            r#"<fragment name="a"><boundary name="ready">ready</boundary></fragment><for each="x in xs"><render fragment="a"/></for>"#,
            codes::BOUNDARY_IN_REPEAT,
        ),
        (
            r#"<fragment name="a"><render fragment="b"/></fragment><fragment name="b"><boundary name="ready">ready</boundary><if condition="again"><render fragment="a"/></if></fragment><for each="x in xs"><render fragment="a"/></for>"#,
            codes::BOUNDARY_IN_REPEAT,
        ),
        (
            r#"<fragment name="a"><boundary name="inner">ready</boundary></fragment><boundary name="outer"><render fragment="a"/></boundary>"#,
            codes::NESTED_BOUNDARY,
        ),
        (
            r#"<fragment name="a"><boundary name="ready">ready</boundary></fragment><render fragment="a"/><render fragment="a"/>"#,
            codes::MISSING_BOUNDARY_KEY,
        ),
    ];
    for (body, code) in cases {
        let source = format!("<body>{body}</body>");
        let error = HtmlParser::new()
            .parse("index.html", &source)
            .expect_err(&source);
        assert!(
            matches!(error, ParserError::Template(diagnostic) if diagnostic.error_code() == Some(code)),
            "{source}"
        );
    }
}

#[test]
fn repeated_keyed_fragment_boundary_marks_every_ancestor() {
    let mut parser = HtmlParser::new();
    parser.parse("index.html", r#"<body><fragment name="a"><boundary name="ready" key="{{id}}">ready</boundary></fragment><render fragment="a" scope="left" as="id"/><render fragment="a" scope="right" as="id"/></body>"#).expect("keyed repeated boundary");
    let records = parser.into_fragment_records();
    assert!(records["index.html"].contains_boundary);
    assert!(records[&record_id("index.html", "a")].contains_boundary);
    assert!(records[&record_id("index.html", "a")].fragments.iter().any(|fragment| {
        matches!(&fragment.fragment, Some(Fragment::Boundary(boundary)) if boundary.phase() == BoundaryPhase::Start && boundary.may_repeat && boundary.owner_fragment_id == "index.html")
    }));
}

#[test]
fn named_record_ids_are_injective_and_use_a_reserved_domain() {
    assert_ne!(record_id("owner", "a"), record_id("owner:a", "a"));
    assert_ne!(record_id("owner", "a"), record_id("owner", "b"));
    let id = record_id("index.html", "a");
    let error = HtmlParser::new()
        .parse(&id, "")
        .expect_err("reserved owner");
    assert!(
        matches!(error, ParserError::Template(diagnostic) if diagnostic.error_code() == Some(codes::RESERVED_FRAGMENT_ID))
    );
    let source = format!("<body><for each=\"x in xs\" template=\"{id}\"/></body>");
    assert!(HtmlParser::new().parse("index.html", &source).is_err());
    let mut parser = HtmlParser::new();
    parser.parse("if-1", "<body><fragment name=\"for-1\"/><render fragment=\"for-1\"/><if condition=\"ok\">text</if></body>").expect("disjoint declaration ID");
    assert!(parser.has_fragment(&record_id("if-1", "for-1")));
}

#[test]
fn recursive_tokens_keep_path_local_definition_contexts() {
    let mut parser = HtmlParser::new();
    parser.parse("index.html", r#"<body>
        <fragment name="defined"><style>:root{--tone:red}</style><render fragment="leaf"/></fragment>
        <fragment name="leaf"><style>.leaf{color:var(--tone)}</style><if condition="again"><render fragment="leaf"/></if></fragment>
        <render fragment="defined"/><render fragment="leaf"/>
    </body>"#).expect("parse");
    assert_eq!(parser.take_tokens(), ["tone"]);
}

#[test]
fn fragment_module_entries_follow_invocation_order_and_skip_boundary_calls() {
    let mut parser = HtmlParser::new();
    parser.parse("index.html", r#"<body>
        <fragment name="late"><script type="module" src="/late.js"></script></fragment>
        <fragment name="early"><script type="module" src="/early.js"></script><if condition="again"><render fragment="early"/></if></fragment>
        <fragment name="deferred"><script type="module" src="/deferred.js"></script></fragment>
        <render fragment="early"/><script type="module" src="/middle.js"></script><render fragment="late"/>
        <boundary name="later"><render fragment="deferred"/></boundary>
    </body>"#).expect("parse");
    assert_eq!(
        parser.module_entry_srcs(),
        ["/early.js", "/middle.js", "/late.js"]
    );
}

#[test]
fn fast_plugins_reject_directives_in_entries_components_and_inert_content() {
    for plugin in [Plugin::FastV2, Plugin::FastV3] {
        for source in [
            "<body><fragment name=\"a\"/></body>",
            "<body><template><render fragment=\"a\"/></template></body>",
            "<body><style><fragment name=\"a\"/></style></body>",
        ] {
            let parser_plugin: Box<dyn crate::plugin::ParserPlugin> = match plugin {
                Plugin::FastV2 => Box::new(crate::plugin::fast_v2::FastV2ParserPlugin::new()),
                _ => Box::new(crate::plugin::fast_v3::FastV3ParserPlugin::new()),
            };
            let mut parser = HtmlParser::with_plugin(parser_plugin);
            let error = parser.parse("index.html", source).expect_err(source);
            assert!(
                matches!(error, ParserError::Template(diagnostic) if diagnostic.error_code() == Some(codes::UNSUPPORTED_FRAGMENT_DIRECTIVE))
            );
        }
    }
}

#[test]
fn render_boundary_context_is_checked_transitively() {
    for container in ["table", "tbody", "tr", "select", "x-host"] {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "x-host",
                "<slot></slot>",
                None,
                false,
            ))
            .expect("register host");
        let source = format!(
            r#"<body><fragment name="a"><render fragment="b"/></fragment><fragment name="b"><boundary name="ready">ready</boundary></fragment><{container}><render fragment="a"/></{container}></body>"#
        );
        let error = parser.parse("index.html", &source).expect_err(&source);
        let expected = if container == "x-host" {
            codes::BOUNDARY_CROSSES_SCOPE
        } else {
            codes::BOUNDARY_IN_FOSTER_CONTEXT
        };
        assert!(
            matches!(error, ParserError::Template(diagnostic) if diagnostic.error_code() == Some(expected)),
            "{source}"
        );
    }
}

#[test]
fn route_boundary_can_render_a_local_fragment_but_ignored_route_html_cannot() {
    let mut parser = HtmlParser::new();
    parser
        .component_registry_mut()
        .register_component(ComponentRegistration::new(
            "x-route",
            "<outlet/>",
            None,
            false,
        ))
        .expect("register route");
    parser.parse("index.html", r#"<body><fragment name="a">{{title}}</fragment><route path="/" component="x-route"><boundary name="ready"><render fragment="a"/></boundary></route></body>"#).expect("active route content");
    assert!(parser.has_fragment(&record_id("index.html", "a")));

    let error = parser.parse("second.html", r#"<body><fragment name="a"/><route path="/" component="x-route"><render fragment="a"/></route></body>"#).expect_err("ignored route content");
    assert!(
        matches!(error, ParserError::Template(diagnostic) if diagnostic.error_code() == Some(codes::INVALID_FRAGMENT_PLACEMENT))
    );
}

#[test]
fn reparsing_named_owner_does_not_retain_old_module_resources() {
    let mut parser = HtmlParser::new();
    parser.parse("index.html", r#"<body><fragment name="a"><script type="module" src="/old.js"></script></fragment><render fragment="a"/></body>"#).expect("first");
    assert_eq!(parser.module_entry_srcs(), ["/old.js"]);
    parser.parse("index.html", r#"<body><fragment name="a"><script type="module" src="/new.js"></script></fragment><render fragment="a"/></body>"#).expect("second");
    assert_eq!(parser.module_entry_srcs(), ["/new.js"]);
}

#[test]
fn fast_component_registration_rejects_inert_fragment_directives() {
    for fast_v3 in [false, true] {
        let plugin: Box<dyn crate::plugin::ParserPlugin> = if fast_v3 {
            Box::new(crate::plugin::fast_v3::FastV3ParserPlugin::new())
        } else {
            Box::new(crate::plugin::fast_v2::FastV2ParserPlugin::new())
        };
        let mut parser = HtmlParser::with_plugin(plugin);
        let error = parser.component_registry_mut().register_component(ComponentRegistration::new(
                    "x-fast",
                    r#"<f-template name="x-fast"><template><template><render fragment="a"/></template></template></f-template>"#,
                    None, true,
                )).expect_err("unsupported FAST source");
        assert!(
            matches!(error, ParserError::Template(diagnostic) if diagnostic.error_code() == Some(codes::UNSUPPORTED_FRAGMENT_DIRECTIVE))
        );
    }
}

#[test]
fn named_directive_casing_matches_existing_webui_directives() {
    let mut parser = HtmlParser::new();
    parser.parse("index.html", r#"<body><FRAGMENT name="ordinary">content</FRAGMENT><RENDER fragment="ordinary"/></body>"#).expect("uppercase tags remain ordinary HTML");
    let records = parser.into_fragment_records();
    assert!(!records
        .values()
        .flat_map(|list| &list.fragments)
        .any(|fragment| { matches!(&fragment.fragment, Some(Fragment::Render(_))) }));
    let mut parser =
        HtmlParser::with_plugin(Box::new(crate::plugin::fast_v3::FastV3ParserPlugin::new()));
    let error = parser
        .parse("index.html", "<body><RENDER fragment=\"a\"/></body>")
        .expect_err("FAST names are case-insensitive");
    assert!(
        matches!(error, ParserError::Template(diagnostic) if diagnostic.error_code() == Some(codes::UNSUPPORTED_FRAGMENT_DIRECTIVE))
    );
}

#[test]
fn bare_table_render_ranges_receive_explicit_shared_containers() {
    for (body, container) in [
        ("<tr><td>{{value}}</td></tr>", "tbody"),
        ("<col>", "colgroup"),
        (
            "<style>td{color:red}</style><tr><td>{{value}}</td></tr>",
            "tbody",
        ),
        (
            "<tr><td>{{value}}</td></tr><script type=\"application/json\">{}</script>",
            "tbody",
        ),
    ] {
        let source = format!("<body><fragment name=\"row\">{body}</fragment><table><render fragment=\"row\"/><render fragment=\"row\"/></table></body>");
        let mut parser = HtmlParser::new();
        parser
            .parse("index.html", &source)
            .expect("parse table fragment");
        let records = parser.into_fragment_records();
        let root = &records["index.html"].fragments;
        let first = root
            .iter()
            .position(|fragment| matches!(&fragment.fragment, Some(Fragment::Render(_))))
            .expect("render");
        assert!(
            matches!(&root[first - 1].fragment, Some(Fragment::Raw(raw)) if raw.value.ends_with(&format!("<table><{container}>")))
        );
        assert!(matches!(
            &root[first + 1].fragment,
            Some(Fragment::Render(_))
        ));
        assert!(
            matches!(&root[first + 2].fragment, Some(Fragment::Raw(raw)) if raw.value.starts_with(&format!("</{container}></table>")))
        );
    }
}

#[test]
fn shared_token_graphs_visit_each_relevant_definition_context_once() {
    let mut source = String::from("<body>");
    for index in 0..12 {
        source.push_str(&format!(
            "<fragment name=\"f{index}\"><style>:root{{--unused{index}:red}}</style><render fragment=\"f{next}\"/><render fragment=\"f{next}\"/></fragment>",
            next = index + 1,
        ));
    }
    source.push_str("<fragment name=\"f12\"><style>p{color:var(--tone)}</style></fragment><render fragment=\"f0\"/></body>");
    let mut parser = HtmlParser::new();
    parser.parse("index.html", &source).expect("shared graph");
    let analysis = parser.token_analysis();
    assert_eq!(analysis.protocol_tokens, ["tone"]);
    assert_eq!(analysis.fallback_chains.len(), 1);
}

#[test]
fn module_entries_survive_fragment_discovery_inside_active_subrecords() {
    for body in [
        r#"<for each="item in items"><script type="module" src="/before.js"></script><x-child></x-child><script type="module" src="/after.js"></script></for>"#,
        r#"<if condition="ready"><script type="module" src="/before.js"></script><x-child></x-child><script type="module" src="/after.js"></script></if>"#,
    ] {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "x-child",
                "<fragment name=\"unused\"/>",
                None,
                false,
            ))
            .expect("register child");
        let source =
            format!("<body><script type=\"module\" src=\"/root.js\"></script>{body}</body>");
        parser.parse("index.html", &source).expect("entry");
        assert_eq!(
            parser.module_entry_srcs(),
            ["/root.js", "/before.js", "/after.js"],
            "{body}"
        );
    }
}

#[test]
fn unused_registered_fragment_owners_do_not_hide_ordinary_module_entries() {
    let mut parser = HtmlParser::new();
    parser
        .component_registry_mut()
        .register_component(ComponentRegistration::new(
            "x-unused",
            "<fragment name=\"unused\"/>",
            None,
            false,
        ))
        .expect("register unused child");
    parser
        .parse(
            "index.html",
            "<body><script type=\"module\" src=\"/entry.js\"></script></body>",
        )
        .expect("ordinary entry");
    assert_eq!(parser.module_entry_srcs(), ["/entry.js"]);
}

#[test]
fn unparsed_registrations_do_not_initialize_module_tracking() {
    let entry = "<body><if condition=\"ready\"><for each=\"item in items\"><p>{{item}}</p></for></if></body>";
    let mut ordinary = HtmlParser::new();
    ordinary.parse("index.html", entry).expect("ordinary entry");
    let expected = ordinary.into_fragment_records();
    for source in [
        "<fragment name=\"unused\"/>",
        "<render fragment=\"missing\"/>",
        "<FRAGMENT name=\"ordinary\">content</FRAGMENT>",
        "<!-- <render fragment=\"ignored\"/> -->",
        r#"<div title='<render fragment="quoted"/>'>content</div>"#,
        r#"<script type="application/json">"<render fragment='raw'/>"</script>"#,
    ] {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new("x-unused", source, None, false))
            .expect("register unparsed owner");
        parser.parse("index.html", entry).expect("ordinary entry");
        assert!(!parser.has_named_fragments, "{source}");
        assert!(parser.module_entry_sites.is_none(), "{source}");
        assert_eq!(parser.into_fragment_records(), expected, "{source}");
    }
}

#[test]
fn child_owners_initialize_module_tracking_inside_untracked_subrecords() {
    for children in [
        "<x-ordinary></x-ordinary><x-named></x-named>",
        "<x-named></x-named><x-ordinary></x-ordinary>",
    ] {
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        for (tag, source) in [
            (
                "x-ordinary",
                r#"<script type="module" src="/ordinary.js"></script>"#,
            ),
            (
                "x-named",
                r#"<render fragment="script"/><fragment name="script"><script type="module" src="/named.js"></script></fragment>"#,
            ),
        ] {
            parser
                .component_registry_mut()
                .register_component(ComponentRegistration::new(tag, source, None, false))
                .expect("register child owner");
        }
        assert!(parser.module_entry_sites.is_none());
        let source = format!(
            "<body><if condition=\"ready\"><for each=\"item in items\">{children}</for></if></body>"
        );
        parser.parse("index.html", &source).expect("nested owners");
        let expected = if children.starts_with("<x-ordinary>") {
            ["/ordinary.js", "/named.js"]
        } else {
            ["/named.js", "/ordinary.js"]
        };
        assert_eq!(parser.module_entry_srcs(), expected, "{children}");
        assert!(parser.current_record_id.is_empty());
        let sites = parser.module_entry_sites.as_ref().expect("module sites");
        assert!(sites.contains_key("x-ordinary"));
        assert!(sites.contains_key(&record_id("x-named", "script")));
        assert!(!sites.contains_key("index.html"));
    }
}

#[test]
fn module_entries_survive_fragment_registration_after_an_ordinary_parse() {
    let mut parser = HtmlParser::with_options(DomStrategy::Light);
    parser
        .component_registry_mut()
        .register_component(ComponentRegistration::new(
            "x-leaf",
            "<i>Leaf</i><script type=\"module\" src=\"/leaf.js\"></script>",
            None,
            false,
        ))
        .expect("register leaf");
    parser
        .component_registry_mut()
        .register_component(ComponentRegistration::new(
            "x-old",
            "<script type=\"module\" src=\"/before.js\"></script><x-leaf></x-leaf><script type=\"module\" src=\"/old.js\"></script>",
            None,
            false,
        ))
        .expect("register ordinary child");
    parser
        .parse("first.html", "<body><x-old></x-old></body>")
        .expect("ordinary parse");
    parser
        .component_registry_mut()
        .register_component(ComponentRegistration::new(
            "x-new",
            "<fragment name=\"unused\"/><script type=\"module\" src=\"/new.js\"></script>",
            None,
            false,
        ))
        .expect("register fragment child");
    parser
        .parse("second.html", "<body><x-old></x-old><x-new></x-new></body>")
        .expect("fragment parse");
    assert_eq!(
        parser.module_entry_srcs(),
        ["/before.js", "/leaf.js", "/old.js", "/new.js"]
    );
}

#[test]
fn entries_accept_declarations_at_the_implicit_body_root() {
    let body =
        "<fragment name=\"heading\"><h2>{{title}}</h2></fragment><render fragment=\"heading\"/>";
    for source in [
        body.to_string(),
        format!("<!DOCTYPE html>{body}"),
        format!("<html><head><title>Page</title></head>{body}</html>"),
        format!("<body>{body}</body>"),
    ] {
        let mut parser = HtmlParser::new();
        parser.parse("index.html", &source).expect("entry root");
        assert!(parser.has_fragment(&record_id("index.html", "heading")));
    }
    for source in [
        format!("<head>{body}</head>"),
        format!("<div>{body}</div>"),
        format!("<html>{body}<body></body></html>"),
    ] {
        let error = HtmlParser::new()
            .parse("index.html", &source)
            .expect_err("not the effective body root");
        assert!(
            matches!(error, ParserError::Template(diagnostic) if diagnostic.error_code() == Some(codes::INVALID_FRAGMENT_PLACEMENT)),
            "{source}"
        );
    }
}

#[test]
fn module_site_tracking_preserves_ordinary_inline_style_ownership() {
    let mut parser = HtmlParser::new();
    parser
        .parse(
            "index.html",
            concat!(
                "<body><script type=\"module\" src=\"/entry.js\"></script>",
                "<for each=\"item in items\"><style>:root{--tone:red}</style></for>",
                "<style>.outside{color:var(--tone)}</style></body>",
            ),
        )
        .expect("ordinary entry");
    assert!(parser.take_tokens().is_empty());
}
