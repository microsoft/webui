// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{
    benchmark_state, hydration_handler, occurrences, render_legacy, render_streaming, BenchWriter,
    ENTRY_ID,
};
use criterion::{BatchSize, Criterion, Throughput};
use std::hint::black_box;
use webui_handler::Protocol;
use webui_parser::plugin::{webui::WebUIParserPlugin, ParserPluginArtifacts};
use webui_parser::{ComponentRegistration, CssStrategy, HtmlParser};
use webui_protocol::{ComponentData, InitialStateStrategy, StateProjectionMode, WebUIProtocol};

const INLINE_CSS_BOUNDARIES: usize = 6 * INLINE_CSS_COMPONENTS.len();
const INLINE_CSS_REPEATS: usize = 4;
const INLINE_CSS_COMPONENTS: &[(&str, &str, &str)] = &[
    (
        "bench-metric",
        r#"<section class="metric"><h2>{{title}}</h2><strong>{{count}}</strong></section>"#,
        r#":host { display: block; min-inline-size: 0; color: var(--metric-ink, #21324a); }
.metric { display: grid; gap: .5rem; padding: 1rem; border: 1px solid #d5deeb;
          border-radius: .75rem; background: linear-gradient(150deg, #fff, #edf5ff); }
h2 { margin: 0; font-size: .875rem; overflow-wrap: anywhere; }
strong { font-size: clamp(1.25rem, 3vw, 2rem); font-variant-numeric: tabular-nums; }
@media (max-width: 40rem) { .metric { padding: .75rem; } }"#,
    ),
    (
        "bench-trend",
        r#"<label class="trend">{{title}}<progress value="{{count}}" max="100"></progress></label>"#,
        r#":host { display: block; color: var(--trend-ink, #1a3360); }
.trend { display: grid; grid-template-columns: minmax(0, 1fr) minmax(4rem, 35%);
         align-items: center; gap: .75rem; padding: .875rem; background: #eaf2ff; }
progress { inline-size: 100%; block-size: .625rem; accent-color: #2764bc; }
@media (max-width: 40rem) { .trend { grid-template-columns: 1fr; gap: .375rem; } }
@media (forced-colors: active) { .trend { border: 1px solid CanvasText; } }"#,
    ),
    (
        "bench-status",
        r#"<p class="status"><span>Active</span> {{title}}</p>"#,
        r#":host { display: block; min-inline-size: 0; }
.status { display: flex; flex-wrap: wrap; align-items: center; gap: .5rem;
          margin: 0; padding: .75rem; color: #233039; background: #f2f7f5; }
span { padding: .25rem .625rem; border-radius: 999px; background: #d3eee3;
       color: #09513c; font-weight: 600; white-space: nowrap; }
span::before { content: ""; display: inline-block; inline-size: .5rem;
               block-size: .5rem; margin-inline-end: .375rem; background: currentColor;
               border-radius: 50%; }"#,
    ),
    (
        "bench-action",
        r#"<button type="button" class="action">Open {{title}}</button>"#,
        r#":host { display: block; }
.action { inline-size: 100%; min-block-size: 2.75rem; padding: .625rem .875rem;
          border: 1px solid #1e467d; border-radius: .5rem; background: #23528f;
          color: white; font: inherit; text-align: start; cursor: pointer; }
.action:hover { background: #1b4275; }
.action:focus-visible { outline: 2px solid #176dd4; outline-offset: 3px; }
.action:disabled { opacity: .6; cursor: default; }
@media (forced-colors: active) { .action:focus-visible { outline-color: Highlight; } }"#,
    ),
];

// Keep hosts in live HTML flow: this workload measures style reuse, not the
// intentional output differences for styles placed in inert markup.
fn inline_css_document() -> WebUIProtocol {
    let mut parser =
        HtmlParser::with_plugin_options(Box::new(WebUIParserPlugin::new()), CssStrategy::Style);
    for &(tag, template, css) in INLINE_CSS_COMPONENTS {
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(tag, template, Some(css), true))
            .unwrap_or_else(|error| panic!("registering <{tag}> failed: {error}"));
    }

    let mut html = String::new();
    html.push_str(
        "<!doctype html><html><head>\
         <script type=\"module\" async src=\"/index.js\"></script>\
         </head><body>",
    );
    for boundary in 0..INLINE_CSS_BOUNDARIES {
        html.push_str(&format!(
            "<boundary name=\"dashboard-{boundary}\"><section>"
        ));
        // Rotate pairs so the first checkpoint introduces two resources, the
        // next two introduce one each, and later checkpoints only reuse them.
        // Repeating the pair also exercises dedup within a single checkpoint.
        for _ in 0..INLINE_CSS_REPEATS {
            for index in boundary..boundary + 2 {
                let (tag, _, _) = INLINE_CSS_COMPONENTS[index % INLINE_CSS_COMPONENTS.len()];
                html.push_str(&format!("<{tag}></{tag}>"));
            }
        }
        html.push_str("</section></boundary>");
    }
    html.push_str("</body></html>");
    parser
        .parse(ENTRY_ID, &html)
        .unwrap_or_else(|error| panic!("parsing inline-CSS benchmark entry failed: {error}"));

    let artifacts = match parser.take_plugin_artifacts() {
        Ok(ParserPluginArtifacts::ComponentTemplates(artifacts)) => artifacts,
        Ok(ParserPluginArtifacts::None) => panic!("missing inline-CSS benchmark templates"),
        Err(error) => panic!("compiling inline-CSS benchmark templates failed: {error}"),
    };
    let components = artifacts
        .into_iter()
        .map(|artifact| {
            let css = parser
                .component_registry()
                .get(&artifact.tag_name)
                .and_then(|component| component.css_content.as_ref())
                .cloned()
                .unwrap_or_else(|| panic!("missing compiled CSS for <{}>", artifact.tag_name));
            (
                artifact.tag_name,
                ComponentData {
                    css,
                    uses_shadow_dom: artifact.uses_shadow_dom,
                    template_json: artifact.template_json,
                    // This fixture's client surface uses only the authored bindings.
                    hydration_mode: StateProjectionMode::Keys as i32,
                    hydration_keys: artifact.template_roots,
                    ..Default::default()
                },
            )
        })
        .collect();
    let mut document = WebUIProtocol::new(parser.into_fragment_records());
    document.set_css_strategy(webui_protocol::CssStrategy::Style);
    document.initial_state_strategy = InitialStateStrategy::Components as i32;
    document.components = components;
    document.populate_style_closures(&[ENTRY_ID]);
    document
}

fn verify_inline_css_output(writer: &BenchWriter, progressive: bool) {
    const STYLE_SOURCE: &str = " data-ws-style>";
    const CSS_NULL: &str = "\"css\":null";
    const ENVELOPE: &str = "data-webui-boundary>[";
    let resources = INLINE_CSS_COMPONENTS.len();
    let hosts = INLINE_CSS_BOUNDARIES * 2 * INLINE_CSS_REPEATS;
    let (mode, expected_sources, expected_envelopes, expected_flushes) = if progressive {
        (
            "progressive",
            resources,
            INLINE_CSS_BOUNDARIES + 1,
            INLINE_CSS_BOUNDARIES + 2,
        )
    } else {
        ("ordinary", 0, 0, 0)
    };
    let output = &writer.output;
    let styles = occurrences(output, r#" data-webui-strategy="style""#);
    let sources = occurrences(output, STYLE_SOURCE);
    let nulls = occurrences(output, CSS_NULL);
    let envelopes = occurrences(output, ENVELOPE);
    let flushes = writer.flushes.len();

    assert_eq!(styles, hosts, "{mode}: every Shadow host needs local CSS");
    assert_eq!(sources, expected_sources, "{mode}: style source markers");
    assert_eq!(nulls, expected_sources, "{mode}: null catalog references");
    assert_eq!(
        occurrences(output, r#""kind":"style""#),
        resources,
        "{mode}: each distinct resource must be published once"
    );
    assert_eq!(envelopes, expected_envelopes, "{mode}: hydration envelopes");
    assert_eq!(
        flushes, expected_flushes,
        "{mode}: shell/checkpoint/terminal flushes"
    );
    for &(tag, _, _) in INLINE_CSS_COMPONENTS {
        let style = format!(r#"<style data-webui-resource="{tag}" data-webui-strategy="style""#);
        assert_eq!(
            occurrences(output, &style),
            hosts / resources,
            "{mode}: {tag} installs"
        );
        assert_eq!(
            occurrences(output, &format!("{style}{STYLE_SOURCE}")),
            usize::from(progressive),
            "{mode}: {tag} can source the catalog at most once"
        );
    }

    // The shell flush precedes the first checkpoint. Every following flush
    // transfers a single record, with source/null pairs only at first use.
    assert!(
        writer.flushes.first().is_none_or(|&end| end > 0),
        "the shell must flush nonempty markup"
    );
    for (checkpoint, offsets) in writer.flushes.windows(2).enumerate() {
        let chunk = &output[offsets[0]..offsets[1]];
        let new_resources = if checkpoint == 0 {
            2
        } else {
            usize::from(checkpoint + 1 < resources)
        };
        assert_eq!(occurrences(chunk, ENVELOPE), 1, "checkpoint {checkpoint}");
        assert_eq!(
            occurrences(chunk, STYLE_SOURCE),
            new_resources,
            "checkpoint {checkpoint}: sources"
        );
        assert_eq!(
            occurrences(chunk, CSS_NULL),
            new_resources,
            "checkpoint {checkpoint}: nulls"
        );
    }

    // Fixed FNV-1a over the exact wire bytes, not a process-seeded hash or a
    // digest of reserialized HTML/JSON. Compare the same mode across revisions.
    let hash = output
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });
    println!(
        "streaming_inline_css {mode}: boundaries={INLINE_CSS_BOUNDARIES}, hosts={hosts}, resources={resources}, bytes={}, fnv1a64={hash:016x}, styles={styles}, style_sources={sources}, css_nulls={nulls}, envelopes={envelopes}, flushes={flushes}",
        output.len(),
    );
}

pub(super) fn bench_inline_css(c: &mut Criterion) {
    let document = inline_css_document();
    let protocol = Protocol::new(document.clone());
    let handler = hydration_handler();
    let state = benchmark_state();
    let mut ordinary = BenchWriter::new(0);
    let mut progressive = BenchWriter::new(INLINE_CSS_BOUNDARIES + 2);

    // Both paths render the same compiled page. Ordinary render ignores the
    // boundary signals. Preflight also grows the sinks outside the timed loop.
    render_legacy(&handler, &protocol, &state, &mut ordinary);
    render_streaming(&handler, &protocol, &state, &mut progressive);
    verify_inline_css_output(&ordinary, false);
    verify_inline_css_output(&progressive, true);

    let mut group = c.benchmark_group("streaming_inline_css");
    group.throughput(Throughput::Bytes(ordinary.output.len() as u64));
    group.bench_function("ordinary", |b| {
        b.iter(|| {
            ordinary.reset();
            render_legacy(
                &handler,
                black_box(&protocol),
                black_box(&state),
                &mut ordinary,
            );
            black_box(ordinary.output.len());
        });
    });
    group.throughput(Throughput::Bytes(progressive.output.len() as u64));
    group.bench_function("progressive", |b| {
        b.iter(|| {
            progressive.reset();
            render_streaming(
                &handler,
                black_box(&protocol),
                black_box(&state),
                &mut progressive,
            );
            black_box((progressive.output.len(), progressive.flushes.len()));
        });
    });
    group.finish();

    // Measure cold preparation without timing compilation or the input clone.
    c.bench_function("streaming_inline_css_prepare_protocol", |b| {
        b.iter_batched(
            || document.clone(),
            |document| black_box(Protocol::new(black_box(document))),
            BatchSize::PerIteration,
        );
    });
    c.bench_function("streaming_inline_css_prepare_and_first_stream", |b| {
        b.iter_batched(
            || document.clone(),
            |document| {
                progressive.reset();
                let protocol = Protocol::new(black_box(document));
                render_streaming(&handler, &protocol, &state, &mut progressive);
                black_box(progressive.output.len());
                black_box(protocol)
            },
            BatchSize::PerIteration,
        );
    });
}
