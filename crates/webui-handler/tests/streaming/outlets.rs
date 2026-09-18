// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{document, parsed_protocol, parsed_protocol_data, FlushStringWriter};
use std::sync::Arc;
use webui_handler::{
    BoundaryMode, HandlerError, Protocol, RenderOptions, ResponseWriter, SessionOptions,
    StreamingSession, WebUIHandler,
};
use webui_protocol::web_ui_fragment::Fragment;
use webui_test_utils::test_json;

#[path = "outlet_markers.rs"]
mod markers;
#[path = "outlet_policy.rs"]
mod policy;

const CHILDREN: &str = concat!(
    r#"<route path="a" component="outlet-a" exact />"#,
    r#"<route path="b" component="outlet-b" exact />"#,
    r#"<route path="c" component="outlet-c" exact />"#,
);

fn outlet_protocol(children: &str, shell: &str, selected: &str) -> Arc<Protocol> {
    parsed_protocol(
        &document(&format!(
            r#"<route path="/" component="outlet-shell">{children}</route>"#
        )),
        &[
            ("outlet-shell", shell),
            ("outlet-a", "<p>A</p>"),
            ("outlet-b", selected),
            ("outlet-c", "<p>C</p>"),
        ],
    )
}

fn outlet_protocol_with_content(
    entry: &str,
    components: &[(&str, &str)],
    route_component: &str,
    content: &str,
) -> Arc<Protocol> {
    // Authored routes retain only direct boundary children as content. Compile
    // a boundary-free body, then replace its fixture-only call with a route edge.
    let mut data = parsed_protocol_data(
        &document(&format!(
            r#"<fragment name="outlet-content">{content}</fragment>{entry}<render fragment="outlet-content" />"#
        )),
        components,
    );
    let entry_record = data.fragments.get_mut("index.html").unwrap();
    let call_index = entry_record
        .fragments
        .iter()
        .rposition(|fragment| matches!(fragment.fragment.as_ref(), Some(Fragment::Render(_))))
        .unwrap();
    let Some(Fragment::Render(call)) = entry_record.fragments.remove(call_index).fragment else {
        panic!("expected the fixture's route-content source call");
    };
    assert!(!data.fragments[&call.fragment_id].contains_boundary);
    if let Some(carrier) = data.fragments.get("outlet-carrier") {
        assert!(!carrier.contains_boundary);
    }
    {
        let mut matching = data
            .fragments
            .values_mut()
            .flat_map(|list| &mut list.fragments)
            .filter_map(|fragment| match fragment.fragment.as_mut() {
                Some(Fragment::Route(route)) if route.fragment_id == route_component => Some(route),
                _ => None,
            });
        let route = matching.next().unwrap();
        assert!(route.content_fragment_id.is_empty());
        assert_eq!(route.children.len(), 3);
        route.content_fragment_id = call.fragment_id;
        assert!(matching.next().is_none());
    }
    data.populate_style_closures(&["index.html"]);
    Arc::new(Protocol::new(data))
}

fn handler() -> WebUIHandler {
    WebUIHandler::with_plugin(
        || Box::new(webui_handler::plugin::webui::WebUIHydrationPlugin::new()),
    )
}

fn ordinary(protocol: &Protocol, path: &str) -> String {
    let mut writer = FlushStringWriter::default();
    handler()
        .render(
            protocol,
            &test_json!({}),
            &RenderOptions::new("index.html", path),
            &mut writer,
        )
        .unwrap();
    writer.output
}

fn routes(html: &str) -> Vec<(&str, &str, bool)> {
    html.split("<webui-route path=\"")
        .skip(1)
        .map(|route| {
            let (path, rest) = route.split_once('"').unwrap();
            let opening = rest.split_once('>').unwrap().0;
            let component = opening
                .split_once(" component=\"")
                .unwrap()
                .1
                .split_once('"')
                .unwrap()
                .0;
            (path, component, opening.ends_with(" active"))
        })
        .collect()
}

fn assert_content_before_host(output: &str, content: &str, host: &str) {
    let content_end = output.find(content).unwrap() + content.len();
    let host_start = output.find(&format!("<{host}")).unwrap();
    assert!(content_end <= host_start, "{output}");
}

#[test]
fn ordinary_outlets_emit_the_winner_first_and_hidden_siblings_in_declaration_order() {
    let protocol = outlet_protocol(CHILDREN, "<main><outlet /></main>", "<p>B</p>");
    for (path, expected) in [
        (
            "/a",
            [
                ("a", "outlet-a", true),
                ("b", "outlet-b", false),
                ("c", "outlet-c", false),
            ],
        ),
        (
            "/b",
            [
                ("b", "outlet-b", true),
                ("a", "outlet-a", false),
                ("c", "outlet-c", false),
            ],
        ),
        (
            "/c",
            [
                ("c", "outlet-c", true),
                ("a", "outlet-a", false),
                ("b", "outlet-b", false),
            ],
        ),
        (
            "/missing",
            [
                ("a", "outlet-a", false),
                ("b", "outlet-b", false),
                ("c", "outlet-c", false),
            ],
        ),
    ] {
        let output = ordinary(&protocol, path);
        assert_eq!(&routes(&output)[1..], &expected, "{path}: {output}");
    }
}

#[test]
fn ordinary_outlet_ties_keep_the_first_matching_declaration() {
    let protocol = outlet_protocol(
        concat!(
            r#"<route path="elsewhere" component="outlet-c" exact />"#,
            r#"<route path=":left" component="outlet-a" exact />"#,
            r#"<route path=":right" component="outlet-b" exact />"#,
        ),
        "<outlet />",
        "<p>B</p>",
    );
    let output = ordinary(&protocol, "/value");
    assert_eq!(
        &routes(&output)[1..],
        &[
            (":left", "outlet-a", true),
            ("elsewhere", "outlet-c", false),
            (":right", "outlet-b", false),
        ],
        "{output}"
    );
}

#[test]
fn ordinary_outlets_skip_only_the_selected_slot_not_its_component_key() {
    let protocol = outlet_protocol(
        concat!(
            r#"<route path="first" component="outlet-b" exact />"#,
            r#"<route path="second" component="outlet-b" exact />"#,
        ),
        "<outlet />",
        "<p>SELECTED</p>",
    );
    let output = ordinary(&protocol, "/second");
    assert_eq!(
        &routes(&output)[1..],
        &[("second", "outlet-b", true), ("first", "outlet-b", false),],
        "{output}"
    );
    assert_eq!(output.matches("<p>SELECTED</p>").count(), 1, "{output}");
}

#[test]
fn nested_outlets_restore_the_caller_and_repeated_outlets_stay_consumed() {
    let protocol = outlet_protocol(
        concat!(
            r#"<route path="before" component="outlet-a" exact />"#,
            r#"<route path="section" component="outlet-b">"#,
            r#"<route path="before" component="outlet-a" exact />"#,
            r#"<route path="chosen" component="outlet-c" exact />"#,
            r#"<route path="after" component="outlet-a" exact />"#,
            "</route>",
            r#"<route path="after" component="outlet-c" exact />"#,
        ),
        "<main><outlet /><hr><outlet /></main>",
        "<section><outlet /><hr><outlet /></section>",
    );
    let output = ordinary(&protocol, "/section/chosen");
    assert_eq!(
        routes(&output),
        [
            ("/", "outlet-shell", true),
            ("section", "outlet-b", true),
            ("chosen", "outlet-c", true),
            ("before", "outlet-a", false),
            ("after", "outlet-a", false),
            ("before", "outlet-a", false),
            ("after", "outlet-c", false),
        ],
        "{output}"
    );
    assert_eq!(output.matches("<p>C</p>").count(), 1, "{output}");
}

#[test]
fn streamed_outlets_keep_declaration_order_across_resume() {
    let protocol = outlet_protocol(
        CHILDREN,
        concat!(
            "<main><outlet /></main>",
            r#"<boundary name="shell-end"><p>shell</p></boundary>"#,
        ),
        concat!(
            r#"<boundary name="leaf"><p>selected</p></boundary>"#,
            "<footer>selected-tail</footer>",
        ),
    );
    let mut session = StreamingSession::new(
        Arc::new(handler()),
        protocol,
        SessionOptions::new("index.html", "/b"),
    )
    .unwrap();
    let start = session.start(test_json!({})).unwrap();
    let leaf = start.boundary.unwrap();
    assert_eq!(leaf.name.as_ref(), "leaf");
    let mut output = String::from_utf8(start.bytes).unwrap();
    assert_eq!(
        &routes(&output)[1..],
        &[("a", "outlet-a", false), ("b", "outlet-b", true)],
        "{output}"
    );
    let commit = session
        .resume_current(leaf.instance_id, BoundaryMode::Final)
        .unwrap();
    output.push_str(&String::from_utf8(commit.bytes).unwrap());
    let advanced = session.advance().unwrap();
    let shell = advanced.boundary.unwrap();
    assert_eq!(shell.name.as_ref(), "shell-end");
    output.push_str(&String::from_utf8(advanced.bytes).unwrap());
    assert_eq!(
        &routes(&output)[1..],
        &[
            ("a", "outlet-a", false),
            ("b", "outlet-b", true),
            ("c", "outlet-c", false),
        ],
        "{output}"
    );
    let commit = session
        .resume_current(shell.instance_id, BoundaryMode::Final)
        .unwrap();
    output.push_str(&String::from_utf8(commit.bytes).unwrap());
    let tail = session.advance().unwrap();
    assert!(tail.done);
    output.push_str(&String::from_utf8(tail.bytes).unwrap());
    assert_eq!(routes(&output).len(), 4, "{output}");
    assert_eq!(
        output.matches("<footer>selected-tail</footer>").count(),
        1,
        "{output}"
    );
}

#[derive(Default)]
struct FailingSelectedWriter {
    output: String,
    failed: bool,
    writes_after_failure: usize,
    fail_on: Option<&'static str>,
}

impl ResponseWriter for FailingSelectedWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        if self.failed {
            self.writes_after_failure += 1;
            return Err(HandlerError::ClientDisconnected);
        }
        if content.contains(self.fail_on.unwrap_or("SELECTED")) {
            self.failed = true;
            return Err(HandlerError::ClientDisconnected);
        }
        self.output.push_str(content);
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

impl webui_handler::FlushWriter for FailingSelectedWriter {
    fn flush(&mut self) -> webui_handler::Result<()> {
        if self.failed {
            self.writes_after_failure += 1;
            return Err(HandlerError::ClientDisconnected);
        }
        Ok(())
    }
}

#[test]
fn ordinary_outlet_failure_stops_before_any_hidden_sibling() {
    let protocol = outlet_protocol(CHILDREN, "<outlet />", "<p>SELECTED</p>");
    let mut writer = FailingSelectedWriter::default();
    assert!(matches!(
        handler().render(
            &protocol,
            &test_json!({}),
            &RenderOptions::new("index.html", "/b"),
            &mut writer,
        ),
        Err(HandlerError::ClientDisconnected)
    ));
    assert!(writer.failed);
    assert_eq!(writer.writes_after_failure, 0);
    for path in ["a", "c"] {
        assert!(
            !writer
                .output
                .contains(&format!(r#"<webui-route path="{path}""#)),
            "{}",
            writer.output
        );
    }
}
