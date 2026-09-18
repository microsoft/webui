// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput};
use serde_json::Value;
use std::hint::black_box;
use webui::{build, BuildOptions, CssStrategy, Plugin, WebUIHandler};
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::{Protocol, RenderOptions};

use super::{
    build_contact_state, contact_book_app_dir, BenchFixture, BenchWriter, BASE_HTML_BYTES,
    BYTES_PER_CONTACT, CONTACT_COUNTS, MEASUREMENT_TIME, REQUEST_PATH, WRITER_HEADROOM,
};

const GROUP: &str = "contact_book_contacts_render_webui_plugin";
const COMPILE_GROUP: &str = "contact_book_compile_webui_plugin";
const COMPILE_CASE: &str = "hot_filesystem_end_to_end";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WebUIGroup {
    Render,
    Compile,
}

fn build_options() -> BuildOptions {
    // The real example has no projection manifest: benchmark full state, not
    // a synthetic projection that omits the contact collections.
    BuildOptions {
        app_dir: contact_book_app_dir(),
        entry: "index.html".to_string(),
        css: CssStrategy::Style,
        plugin: Some(Plugin::WebUI),
        ..BuildOptions::default()
    }
}

fn validate_compiled_protocol(protocol: &webui::WebUIProtocol) {
    assert_eq!(
        protocol.initial_state_strategy,
        webui_protocol::InitialStateStrategy::Full as i32,
        "the actual example must retain full bootstrap state"
    );
    assert!(
        protocol
            .components
            .get("cb-app")
            .is_some_and(|component| !component.template_json.is_empty()),
        "WebUI compilation must produce the cb-app template"
    );
}

fn setup() -> BenchFixture {
    let built = build(build_options())
        .unwrap_or_else(|error| panic!("WebUI contact-book build failed: {error}"));
    validate_compiled_protocol(&built.protocol);

    BenchFixture {
        protocol: Protocol::new(built.protocol),
        protocol_bytes: built.protocol_bytes,
        states: CONTACT_COUNTS
            .iter()
            .map(|&count| (count, build_contact_state(count)))
            .collect(),
    }
}

pub(super) fn compile_bench(c: &mut Criterion) {
    let options = build_options();
    assert!(options.projection_manifests.is_empty());
    let warmup =
        build(options).unwrap_or_else(|error| panic!("WebUI compile warmup failed: {error}"));
    validate_compiled_protocol(&warmup.protocol);
    assert!(!warmup.protocol_bytes.is_empty());
    println!(
        "webui_compile hot_filesystem_end_to_end: protocol_bytes={}, components={}, fragments={}",
        warmup.protocol_bytes.len(),
        warmup.stats.component_count,
        warmup.stats.fragment_count,
    );
    drop(warmup);

    let mut group = c.benchmark_group(COMPILE_GROUP);
    group.measurement_time(MEASUREMENT_TIME);
    group.throughput(Throughput::Elements(1));
    group.bench_function(COMPILE_CASE, |b| {
        // Every timed call constructs and consumes fresh options and runs the
        // normal public build API. PerIteration excludes result destruction
        // without accumulating an unbounded batch of compiled applications.
        b.iter_batched(
            || (),
            |()| {
                let built = build(black_box(build_options()))
                    .unwrap_or_else(|error| panic!("WebUI compile failed: {error}"));
                black_box(built)
            },
            BatchSize::PerIteration,
        );
    });
    group.finish();
}

fn handler() -> WebUIHandler {
    WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()))
}

fn validate_bootstrap(html: &str, state: &Value) {
    let (_, script) = html
        .split_once("<script type=\"application/json\" id=\"webui-data\"")
        .unwrap_or_else(|| panic!("WebUI bootstrap script is missing"));
    let (_, content) = script
        .split_once('>')
        .unwrap_or_else(|| panic!("WebUI bootstrap opening tag is incomplete"));
    let (json, _) = content
        .split_once("</script>")
        .unwrap_or_else(|| panic!("WebUI bootstrap closing tag is missing"));
    let bootstrap: Value = serde_json::from_str(json)
        .unwrap_or_else(|error| panic!("invalid WebUI bootstrap JSON: {error}"));
    assert!(
        bootstrap["templates"]
            .as_object()
            .is_some_and(|templates| templates.contains_key("cb-app")),
        "the emitted WebUI bootstrap must contain the cb-app template"
    );
    assert_eq!(
        bootstrap.get("state"),
        Some(state),
        "the WebUI benchmark must serialize the complete, unchanged input state"
    );
}

fn warmup(protocol: &Protocol, state: &Value, count: usize) -> BenchWriter {
    let mut writer = BenchWriter::new(count * BYTES_PER_CONTACT + BASE_HTML_BYTES);
    handler()
        .render(
            protocol,
            state,
            &RenderOptions::new("index.html", REQUEST_PATH),
            &mut writer,
        )
        .unwrap_or_else(|error| panic!("WebUI warmup failed for {count} contacts: {error}"));
    validate_bootstrap(&writer.output, state);
    assert!(
        writer
            .output
            .contains("<h2 class=\"page-title\">All Contacts</h2>"),
        "the benchmark must render the contacts page, not the dashboard"
    );
    writer
}

fn capture_warmup(fixture: &BenchFixture, state: &Value, writer: &BenchWriter, count: usize) {
    let Some(directory) = std::env::var_os("WEBUI_BENCH_CAPTURE_DIR") else {
        return;
    };
    // Preflight only: retain the exact benchmark workload without running an
    // allocator-instrumented memory probe or changing the timed render loop.
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory)
        .unwrap_or_else(|error| panic!("creating WebUI capture directory failed: {error}"));
    let state = serde_json::to_vec(state)
        .unwrap_or_else(|error| panic!("serializing captured WebUI state failed: {error}"));
    for (name, bytes) in [
        (
            "protocol.bin".to_string(),
            fixture.protocol_bytes.as_slice(),
        ),
        (format!("output-{count}.html"), writer.output.as_bytes()),
        (format!("state-{count}.json"), state.as_slice()),
    ] {
        std::fs::write(directory.join(name), bytes)
            .unwrap_or_else(|error| panic!("writing WebUI capture failed: {error}"));
    }
}

pub(super) fn handler_rendering_bench(c: &mut Criterion) {
    let fixture = setup();
    let mut group = c.benchmark_group(GROUP);
    group.measurement_time(MEASUREMENT_TIME);
    let mut previous_bytes = 0;

    for (count, state) in &fixture.states {
        let warmup_writer = warmup(&fixture.protocol, state, *count);
        capture_warmup(&fixture, state, &warmup_writer, *count);
        assert!(
            warmup_writer.len() > previous_bytes,
            "WebUI output must grow with the contact count"
        );
        previous_bytes = warmup_writer.len();
        group.throughput(Throughput::Bytes(warmup_writer.len() as u64));

        group.bench_with_input(BenchmarkId::new("contacts", count), state, |b, state| {
            let h = handler();
            let mut writer = BenchWriter::new(warmup_writer.len() + WRITER_HEADROOM);
            let capacity = writer.output.capacity();

            b.iter(|| {
                writer.clear();
                h.render(
                    black_box(&fixture.protocol),
                    black_box(state),
                    &RenderOptions::new("index.html", REQUEST_PATH),
                    &mut writer,
                )
                .unwrap_or_else(|error| {
                    panic!("WebUI render failed for {count} contacts: {error}")
                });
            });

            // Outside timed iterations; Criterion --test exercises these too.
            assert_eq!(writer.output, warmup_writer.output, "render output drifted");
            assert_eq!(writer.output.capacity(), capacity, "writer buffer grew");
        });
    }

    group.finish();
}

fn webui_filter_group(filter: &str) -> Option<WebUIGroup> {
    let filter = filter.strip_prefix('^').unwrap_or(filter);
    let filter = filter.strip_suffix('$').unwrap_or(filter);
    let filter = filter
        .strip_prefix("(?:")
        .and_then(|inner| inner.strip_suffix(')'))
        .unwrap_or(filter);
    if let Some(suffix) = filter.strip_prefix(COMPILE_GROUP) {
        return (matches!(suffix, "" | "/" | "/.*")
            || suffix.strip_prefix('/') == Some(COMPILE_CASE))
        .then_some(WebUIGroup::Compile);
    }
    let suffix = filter.strip_prefix(GROUP)?;
    if matches!(suffix, "" | "/" | "/contacts" | "/contacts/" | "/.*") {
        return Some(WebUIGroup::Render);
    }
    suffix
        .strip_prefix("/contacts/")
        .and_then(|count| count.parse::<usize>().ok())
        .is_some_and(|count| CONTACT_COUNTS.contains(&count))
        .then_some(WebUIGroup::Render)
}

fn option_takes_value(argument: &str) -> bool {
    matches!(
        argument,
        "--save-baseline"
            | "-b"
            | "--baseline"
            | "--baseline-lenient"
            | "--load-baseline"
            | "--profile-time"
            | "--warm-up-time"
            | "--measurement-time"
            | "--sample-size"
            | "--nresamples"
            | "--noise-threshold"
            | "--confidence-level"
            | "--significance-level"
            | "--plotting-backend"
            | "--output-format"
            | "--color"
            | "--skip"
    )
}

/// Select only a literal WebUI group or one of its exact scenario filters.
/// Mixed/general regex filters keep the existing full-suite/summary behavior.
#[must_use]
pub(super) fn selected_webui_group<I, S>(arguments: I) -> Option<WebUIGroup>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut arguments = arguments.into_iter();
    let mut selected = None;
    while let Some(argument) = arguments.next() {
        let argument = argument.as_ref();
        if argument == "--" {
            if selected.is_some() {
                return None;
            }
            let group = webui_filter_group(arguments.next()?.as_ref())?;
            return arguments.next().is_none().then_some(group);
        }
        // Do not mistake option values (especially baseline names) for filters.
        if option_takes_value(argument) {
            arguments.next()?;
            continue;
        }
        if matches!(
            argument,
            "--bench"
                | "--test"
                | "--list"
                | "--noplot"
                | "--verbose"
                | "-v"
                | "--quiet"
                | "-q"
                | "--exact"
                | "--discard-baseline"
        ) || argument
            .split_once('=')
            .is_some_and(|(option, _)| option_takes_value(option))
        {
            continue;
        }
        if selected.is_some() {
            return None;
        }
        selected = Some(webui_filter_group(argument)?);
    }
    selected
}

fn selects_webui_only<I, S>(arguments: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    selected_webui_group(arguments).is_some()
}

pub(super) fn self_check_selection() {
    assert!(selects_webui_only([GROUP]));
    assert!(selects_webui_only(["--test", GROUP]));
    assert!(selects_webui_only(["--", GROUP]));
    assert!(selects_webui_only([
        "--bench",
        "--measurement-time",
        "1",
        "^contact_book_contacts_render_webui_plugin/contacts/1000$",
    ]));
    assert!(selects_webui_only([
        "^(?:contact_book_contacts_render_webui_plugin/contacts/1000)$",
        "--save-baseline",
        "original",
    ]));
    assert!(!selects_webui_only(["--save-baseline", GROUP]));
    assert!(!selects_webui_only(["--unknown-option", GROUP]));
    assert!(!selects_webui_only(["--skip", GROUP]));
    assert!(!selects_webui_only(["--save-baseline"]));
    assert!(!selects_webui_only(["--test"]));
    assert!(!selects_webui_only(["contact_book_contacts_render"]));
    assert!(!selects_webui_only([
        "contact_book_contacts_render_fast_plugin"
    ]));
    assert!(!selects_webui_only([
        "contact_book_contacts_render_webui_plugin|contact_book_contacts_render$"
    ]));
    assert_eq!(selected_webui_group([GROUP]), Some(WebUIGroup::Render));
    assert_eq!(
        selected_webui_group([COMPILE_GROUP]),
        Some(WebUIGroup::Compile)
    );
    assert!(selects_webui_only(["--test", COMPILE_GROUP]));
    assert_eq!(
        selected_webui_group([
            "--bench",
            "--measurement-time",
            "1",
            "^(?:contact_book_compile_webui_plugin/hot_filesystem_end_to_end)$",
        ]),
        Some(WebUIGroup::Compile)
    );
    assert!(!selects_webui_only(["--save-baseline", COMPILE_GROUP]));
    assert!(!selects_webui_only(["--skip", COMPILE_GROUP]));
    assert!(!selects_webui_only([
        "contact_book_compile_webui_plugin/hot_filesystem_end_to_end_extra"
    ]));
    assert!(!selects_webui_only([
        "contact_book_compile_webui_plugin|contact_book_contacts_render$"
    ]));
}
