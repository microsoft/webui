// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use crate::plugin::webui::WebUIParserPlugin;
use crate::*;

#[test]
fn false_positive_directives_preserve_ordinary_css_ownership() {
    for webui in [false, true] {
        for candidate in [
            "",
            "<!-- <fragment name='ignored'/> -->",
            "<!-- <render fragment='missing'/> -->",
            "<span data-note=\"<fragment name='ignored'/>\"></span>",
            "<span data-note=\"<render fragment='missing'/>\"></span>",
            "<FRAGMENT name='ignored'></FRAGMENT>",
            "<RENDER fragment='missing'></RENDER>",
        ] {
            let mut parser = if webui {
                HtmlParser::with_plugin(Box::new(WebUIParserPlugin::new()))
            } else {
                HtmlParser::new()
            };
            let source = format!(
                "<body>{candidate}<for each=\"item in items\"><style>:root{{--tone:red}}</style></for><style>.outside{{color:var(--tone)}}</style></body>"
            );
            parser.parse("index.html", &source).expect(&source);
            assert!(!parser.has_named_fragments, "{source}");
            assert!(parser.module_entry_sites.is_none(), "{source}");
            assert!(parser.take_tokens().is_empty(), "{source}");
            assert!(parser.fragment_css_tokens.contains_key("index.html"));
            assert!(!parser.fragment_css_tokens.contains_key("for-1"));
        }
    }
}

#[test]
fn empty_graph_filter_keeps_standalone_render_validation() {
    for webui in [false, true] {
        let mut parser = if webui {
            HtmlParser::with_plugin(Box::new(WebUIParserPlugin::new()))
        } else {
            HtmlParser::new()
        };
        let source = "<body>\n<render fragment=\"missing\"/>\n</body>";
        let error = parser
            .parse("index.html", source)
            .expect_err("unknown target");
        let ParserError::Template(diagnostic) = error else {
            panic!("expected template diagnostic");
        };
        assert_eq!(diagnostic.error_code(), Some(codes::UNKNOWN_FRAGMENT));
        assert_eq!(diagnostic.component_name(), Some("index.html"));
        assert_eq!(diagnostic.position_line_column(), Some((2, 1)));
    }
}

#[test]
fn unused_boundary_first_compilation_preserves_critical_component_modules() {
    let deferred = concat!(
        "<fragment name=\"deferred\"><script type=\"module\" src=\"/unused.js\"></script>",
        "<boundary name=\"later\"><script type=\"module\" src=\"/boundary.js\"></script>",
        "<x-shared></x-shared></boundary></fragment>",
    );
    let used = "<fragment name=\"used\"><x-shared></x-shared></fragment>";
    for webui in [false, true] {
        for declarations in [format!("{deferred}{used}"), format!("{used}{deferred}")] {
            let mut parser = if webui {
                HtmlParser::with_plugin(Box::new(WebUIParserPlugin::new()))
            } else {
                HtmlParser::new()
            };
            parser
                .component_registry_mut()
                .register_component(ComponentRegistration::new(
                    "x-shared",
                    concat!(
                        "<script type=\"module\" src=\"/shared.js\"></script>",
                        "<if condition=\"ready\"><script type=\"module\" src=\"/nested.js\"></script></if>",
                    ),
                    None,
                    false,
                ))
                .expect("register shared owner");
            let source = format!(
                "<body>{declarations}<boundary name=\"outer\"><render fragment=\"used\"/></boundary><script type=\"module\" src=\"/before.js\"></script><render fragment=\"used\"/><render fragment=\"used\"/><script type=\"module\" src=\"/after.js\"></script></body>"
            );
            parser
                .parse("index.html", &source)
                .expect("reused component graph");
            assert_eq!(
                parser.module_entry_srcs(),
                ["/before.js", "/shared.js", "/nested.js", "/after.js"]
            );
            assert!(!parser.has_fragment(&named_fragments::record_id("index.html", "deferred")));
        }
    }
}

#[test]
fn boundary_only_module_sites_remain_deferred_in_named_graphs() {
    let mut parser = HtmlParser::new();
    parser
        .parse(
            "index.html",
            concat!(
                "<body><fragment name=\"deferred\"><script type=\"module\" src=\"/called.js\"></script></fragment>",
                "<boundary name=\"later\"><script type=\"module\" src=\"/inline.js\"></script>",
                "<if condition=\"ready\"><script type=\"module\" src=\"/nested.js\"></script></if>",
                "<render fragment=\"deferred\"/></boundary></body>",
            ),
        )
        .expect("deferred graph");
    assert!(parser.module_entry_srcs().is_empty());
    let sites = parser
        .module_entry_sites
        .as_ref()
        .expect("module provenance");
    assert!(sites
        .values()
        .flatten()
        .any(|site| site.src == "/inline.js"));
    assert!(sites
        .values()
        .flatten()
        .any(|site| site.src == "/nested.js"));
}

#[test]
fn boundary_first_module_history_survives_late_fragment_registration() {
    let mut parser = HtmlParser::new();
    parser
        .component_registry_mut()
        .register_component(ComponentRegistration::new(
            "x-shared",
            concat!(
                "<script type=\"module\" src=\"/shared.js\"></script>",
                "<if condition=\"ready\"><script type=\"module\" src=\"/shared.js\"></script></if>",
            ),
            None,
            false,
        ))
        .expect("register ordinary owner");
    parser
        .parse(
            "first.html",
            "<body><boundary name=\"later\"><x-shared></x-shared></boundary></body>",
        )
        .expect("ordinary deferred occurrence");
    assert!(!parser.has_named_fragments);
    assert!(parser.module_entry_srcs().is_empty());
    parser
        .component_registry_mut()
        .register_component(ComponentRegistration::new(
            "x-late",
            "<fragment name=\"part\"><span>late</span></fragment><render fragment=\"part\"/>",
            None,
            false,
        ))
        .expect("register named owner later");
    parser
        .parse(
            "second.html",
            "<body><x-shared></x-shared><x-late></x-late></body>",
        )
        .expect("critical reuse after fragment discovery");
    assert_eq!(parser.module_entry_srcs(), ["/shared.js"]);
}
