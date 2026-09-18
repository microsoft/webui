// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;
use crate::{
    FlushWriter, RenderOptions, StateView, WebUIHandler, WebUiBootstrap, WebUiBootstrapState,
};

crate::define_string_response_writer!(Writer, output);

impl FlushWriter for Writer {
    fn flush(&mut self) -> crate::Result<()> {
        Ok(())
    }
}

fn structural(name: &str) -> WebUIFragment {
    WebUIFragment::signal(format!("{}{name}", crate::STRUCTURAL_SIGNAL_PREFIX), true)
}

fn render_protocol(strategy: CssStrategy) -> WebUIProtocol {
    let mut data = protocol(strategy);
    let mut entry = vec![
        WebUIFragment::raw("<!DOCTYPE html><html><head>"),
        structural("head_start"),
        structural("head_end"),
        WebUIFragment::raw("</head><body>"),
        structural("body_start"),
    ];
    for (declaration, tag) in [(0, "z-card"), (1, "a-card"), (2, "z-card")] {
        entry.extend([
            WebUIFragment::boundary(
                declaration,
                "index.html",
                format!("boundary-{declaration}"),
                None,
            ),
            WebUIFragment::raw(format!("<{tag}")),
            structural(&format!("streaming_root:{tag}")),
            WebUIFragment::raw(">"),
            WebUIFragment::component(tag),
            WebUIFragment::raw(format!("</{tag}>")),
            WebUIFragment::boundary_end(declaration),
        ]);
    }
    entry.extend([structural("body_end"), WebUIFragment::raw("</body></html>")]);
    data.fragments.insert(
        "index.html".to_owned(),
        FragmentList {
            fragments: entry,
            contains_boundary: true,
        },
    );
    data.populate_style_closures(&["index.html"]);
    data
}

fn handler() -> WebUIHandler {
    WebUIHandler::with_plugin(|| Box::new(crate::plugin::webui::WebUIHydrationPlugin::new()))
}

#[derive(serde::Deserialize)]
struct Catalog<'a> {
    #[serde(rename = "componentStyles", borrow)]
    styles: &'a RawValue,
}

fn script_body(html: &str) -> Result<&str, Box<dyn std::error::Error>> {
    let (_, body) = html.split_once('>').ok_or("missing script start")?;
    Ok(body.split_once("</script>").ok_or("missing script end")?.0)
}

fn streaming_catalogs(html: &str) -> Result<Vec<&str>, Box<dyn std::error::Error>> {
    let mut catalogs = Vec::new();
    for script in html
        .split(r#"<script type="application/json" data-webui-boundary"#)
        .skip(1)
    {
        let (_, kind, _, payload): (u32, u32, u32, &RawValue) =
            serde_json::from_str(script_body(script)?)?;
        if kind == 4 {
            continue;
        }
        let catalog: Catalog<'_> = serde_json::from_str(payload.get())?;
        catalogs.push(catalog.styles.get());
    }
    Ok(catalogs)
}

#[test]
fn ordinary_and_streaming_bootstrap_have_exact_catalog_bytes() -> TestResult {
    for strategy in [CssStrategy::Link, CssStrategy::Style, CssStrategy::Module] {
        let runtime = Protocol::new(render_protocol(strategy));
        let options = RenderOptions::new("index.html", "/").with_nonce("payload-nonce");
        let mut ordinary = Writer::with_capacity(4096);
        handler().render(&runtime, &Value::Null, &options, &mut ordinary)?;
        let script = ordinary
            .output
            .split_once(r#"<script type="application/json" id="webui-data""#)
            .ok_or("missing ordinary bootstrap")?
            .1;
        assert!(script.starts_with(r#" nonce="payload-nonce">"#));
        let body = script_body(script)?;
        let catalog: Catalog<'_> = serde_json::from_str(body)?;
        assert_eq!(
            catalog.styles.get(),
            expected(
                strategy,
                r#"{"a-card":["a-card"],"index.html":["z-card","a-card"],"z-card":["z-card"]}"#,
                resources(strategy),
            ),
        );
        assert!(
            body.find("\"componentStyles\"").ok_or("missing styles")?
                < body.find("\"templates\"").ok_or("missing templates")?
        );
        let mut streaming = Writer::with_capacity(4096);
        handler().render_streaming(&runtime, &Value::Null, &options, &mut streaming)?;
        let catalogs = streaming_catalogs(&streaming.output)?;
        assert_eq!(catalogs.len(), 3);
        assert_eq!(
            catalogs[0],
            expected(
                strategy,
                r#"{"index.html":["z-card","a-card"],"z-card":["z-card"]}"#,
                resources(strategy),
            ),
        );
        assert_eq!(
            catalogs[1],
            expected(strategy, r#"{"a-card":["a-card"]}"#, "{}"),
        );
        assert_eq!(catalogs[2], expected(strategy, "{}", "{}"));
        assert!(streaming.output.contains(
            r#"<script type="application/json" data-webui-boundary nonce="payload-nonce">"#,
        ));
    }
    Ok(())
}

fn bootstrap<'a, T: Serialize + ?Sized>(styles: &'a T, state: &'a Value) -> WebUiBootstrap<'a, T> {
    WebUiBootstrap {
        declaration_id: None,
        enclosing_span_instance_id: None,
        state: WebUiBootstrapState::Complete {
            value: StateView::from(state),
            selection: StateSelection::Full,
        },
        owner_props: &[],
        fragment_sources: &[],
        fragment_source_refs: &[],
        chain: &[],
        inventory: "",
        nonce: Some("payload-nonce"),
        css_hrefs: &[],
        style_specs: &[],
        component_styles: styles,
        templates: &[],
    }
}

#[test]
fn borrowed_bootstrap_preserves_script_escaping_nonce_and_null_fields() -> TestResult {
    let mut data = protocol(CssStrategy::Module);
    data.components.get_mut("a-card").ok_or("component")?.css =
        ".a{content:\"</script>😀&\\\"\"}".repeat(300);
    let styles = full_payload(&data, ["a-card"], &HashMap::new())?;
    let owned = collect_component_styles(&data, ["a-card"])?;
    let mut writer = Writer::with_capacity(4096);
    let mut scratch = Vec::new();
    crate::write_webui_data_block(&mut writer, &mut scratch, bootstrap(&styles, &Value::Null))?;
    assert_eq!(
        writer.output,
        format!(
            "<script type=\"application/json\" id=\"webui-data\" nonce=\"payload-nonce\">\
             {{\"componentStyles\":{},\"inventory\":\"\",\"nonce\":\"payload-nonce\",\"state\":null}}\
             </script>\n",
            serde_json::to_string(&owned)?.replace("</", "<\\/"),
        ),
    );
    assert!(scratch.capacity() <= 4096);
    Ok(())
}

#[test]
fn malformed_style_metadata_fails_before_ordinary_or_streaming_output() -> TestResult {
    let mut data = render_protocol(CssStrategy::Style);
    data.components.remove("a-card");
    let runtime = Protocol::new(data);
    let options = RenderOptions::new("index.html", "/");
    let mut ordinary = Writer::with_capacity(0);
    assert!(matches!(
        handler().render(&runtime, &Value::Null, &options, &mut ordinary),
        Err(HandlerError::Invariant(_)),
    ));
    assert!(ordinary.output.is_empty());
    let mut streaming = Writer::with_capacity(0);
    assert!(matches!(
        handler().render_streaming(&runtime, &Value::Null, &options, &mut streaming),
        Err(HandlerError::Invariant(_)),
    ));
    assert!(streaming.output.is_empty());
    Ok(())
}

#[test]
fn bootstrap_propagates_custom_serializer_failure_without_closing_output() -> TestResult {
    struct InvalidStyle;
    impl Serialize for InvalidStyle {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("style serialization rejected"))
        }
    }
    let mut writer = Writer::with_capacity(0);
    let error = crate::write_webui_data_block(
        &mut writer,
        &mut Vec::new(),
        bootstrap(&InvalidStyle, &Value::Null),
    )
    .err()
    .ok_or("serialization succeeded")?;
    assert!(matches!(
        error,
        HandlerError::Rendering(message) if message.contains("style serialization rejected"),
    ));
    assert!(!writer.output.contains("</script>"));
    assert!(!writer.output.contains("\"state\""));
    Ok(())
}
