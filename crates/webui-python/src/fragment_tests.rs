// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;
use webui_protocol::WebUIProtocol;

#[test]
fn named_fragment_bytes_decode_and_render_through_python_binding() {
    let source = include_str!("../../webui-test-utils/fixtures/recursive-fragments.html");
    let state = include_str!("../../webui-test-utils/fixtures/recursive-fragments.json");
    let mut parser = webui_parser::HtmlParser::new();
    parser
        .parse("index.html", source)
        .unwrap_or_else(|error| panic!("build: {error}"));
    let bytes = WebUIProtocol::new(parser.into_fragment_records())
        .to_protobuf()
        .unwrap_or_else(|error| panic!("encode: {error}"));
    Python::initialize();
    Python::attach(|py| {
        let renderer = NativeRenderer::from_bytes(py, &bytes, None)
            .unwrap_or_else(|error| panic!("Python decode: {error}"));
        let input = PyString::new(py, state);
        for _ in 0..2 {
            let output = renderer
                .render(
                    py,
                    input.as_any(),
                    ("index.html".to_string(), "/".to_string(), None, None, None),
                )
                .unwrap_or_else(|error| panic!("Python render: {error}"));
            let html = std::str::from_utf8(output.as_bytes())
                .unwrap_or_else(|error| panic!("UTF-8: {error}"));
            assert!(html.contains("<h2>Tree</h2>"));
            assert!(html.contains(
                "<li><span>Oak</span><ul><li><span>Leaf &amp; bud</span></li></ul></li>"
            ));
            assert_eq!(html.matches("<li>").count(), 3);
            assert!(!html.contains("<fragment"));
        }
    });
}
