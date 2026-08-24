// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use webui_parser::{
    plugin::{fast_v2::FastV2ParserPlugin, ParserPlugin},
    ComponentRegistration, HtmlParser,
};

fn build_attribute_heavy_template(repetitions: usize) -> String {
    let mut html = String::with_capacity(repetitions * 220 + 64);
    html.push_str("<body><div class=\"root\">");
    for idx in 0..repetitions {
        html.push_str("<x-bench-button ");
        html.push_str("class=\"btn {{theme}} {{size}}\" ");
        html.push_str("?disabled=\"{{isDisabled}}\" ");
        html.push_str(":config=\"{{settings}}\" ");
        html.push_str("title=\"item ");
        html.push_str(&idx.to_string());
        html.push_str(" {{tooltip}}\" data-x=\"{{meta}}\">");
        html.push_str("{{label}}</x-bench-button>");
    }
    html.push_str("</div></body>");
    html
}

fn build_ordinary_component_source(depth: usize) -> String {
    let mut html = String::with_capacity(depth * 33 + 64);
    html.push_str("<template>");
    for _ in 0..depth {
        html.push_str("<section class=\"level\">");
    }
    html.push_str("<span>{{title}}</span>");
    for _ in 0..depth {
        html.push_str("</section>");
    }
    html.push_str("</template>");
    html
}

fn build_fast_component_source(depth: usize) -> String {
    let mut html = String::with_capacity(depth * 63 + 128);
    html.push_str("<f-template name=\"x-registration-bench\"><template>");
    for _ in 0..depth {
        html.push_str("<f-when value=\"{{visible}}\"><section>");
    }
    html.push_str("<span>{{title}}</span>");
    for _ in 0..depth {
        html.push_str("</section></f-when>");
    }
    html.push_str("</template></f-template>");
    html
}

fn build_fast_flat_static_source(width: usize) -> String {
    let mut html = String::with_capacity(width * 42 + 128);
    html.push_str("<f-template name=\"x-registration-bench\"><template>");
    for idx in 0..width {
        html.push_str("<section class=\"level\"><span>{{title");
        html.push_str(&idx.to_string());
        html.push_str("}}</span></section>");
    }
    html.push_str("</template></f-template>");
    html
}

fn build_fast_long_attributes_source(attrs: usize) -> String {
    let mut html = String::with_capacity(attrs * 30 + 160);
    html.push_str("<f-template name=\"x-registration-bench\"><template><div");
    for idx in 0..attrs {
        html.push_str(" data-attr-");
        html.push_str(&idx.to_string());
        html.push_str("=\"value-");
        html.push_str(&idx.to_string());
        html.push('"');
    }
    html.push_str(">{{title}}</div></template></f-template>");
    html
}

fn build_fast_large_text_source(bytes: usize) -> String {
    let mut html = String::with_capacity(bytes + 160);
    html.push_str("<f-template name=\"x-registration-bench\"><template><p>");
    while html.len() < bytes {
        html.push_str("Lorem ipsum dolor sit amet. ");
    }
    html.push_str("</p></template></f-template>");
    html
}

fn build_false_positive_ordinary_source(depth: usize) -> String {
    let mut html = String::with_capacity(depth * 33 + 128);
    html.push_str("<template><!-- see f-template docs -->");
    html.push_str("<span data-note=\"f-template\">");
    for _ in 0..depth {
        html.push_str("<section class=\"level\">");
    }
    html.push_str("<span>{{title}}</span>");
    for _ in 0..depth {
        html.push_str("</section>");
    }
    html.push_str("</span></template>");
    html
}

fn parser_with_components(plugin: Option<Box<dyn ParserPlugin>>) -> HtmlParser {
    let mut parser = plugin.map_or_else(HtmlParser::new, HtmlParser::with_plugin);
    let registry = parser.component_registry_mut();
    for tag in ["x-bench-button", "x-card"] {
        registry
            .register_component(ComponentRegistration::new(
                tag,
                r#"<template shadowrootmode="open"><slot></slot></template>"#,
                None,
                true,
            ))
            .unwrap_or_else(|error| panic!("failed to register {tag}: {error}"));
    }
    parser
}

fn parser_plugin_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser_plugin_fast");
    let input = build_attribute_heavy_template(120);
    group.throughput(Throughput::Bytes(input.len() as u64));

    group.bench_function("without_plugin", |b| {
        let mut parser = parser_with_components(None);
        b.iter(|| {
            parser
                .parse("index.html", black_box(&input))
                .unwrap_or_else(|error| panic!("parse without plugin failed: {error}"));
        });
    });

    group.bench_function("with_fast_plugin", |b| {
        let mut parser = parser_with_components(Some(Box::new(FastV2ParserPlugin::new())));
        b.iter(|| {
            parser
                .parse("index.html", black_box(&input))
                .unwrap_or_else(|error| panic!("parse with FAST plugin failed: {error}"));
        });
    });

    group.finish();
}

fn parser_fast_plugin_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("component_registration_fast_source_transform");
    let scenarios = [
        ("ordinary", 8, build_ordinary_component_source(8)),
        ("ordinary", 64, build_ordinary_component_source(64)),
        ("f_template", 8, build_fast_component_source(8)),
        ("f_template", 64, build_fast_component_source(64)),
        ("flat_static", 64, build_fast_flat_static_source(64)),
        ("long_attrs", 64, build_fast_long_attributes_source(64)),
        ("large_text", 2048, build_fast_large_text_source(2048)),
        (
            "false_positive",
            64,
            build_false_positive_ordinary_source(64),
        ),
    ];

    for (source_kind, depth, source) in scenarios {
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(
            BenchmarkId::new(source_kind, depth),
            &source,
            |b, source| {
                b.iter_batched(
                    || HtmlParser::with_plugin(Box::new(FastV2ParserPlugin::new())),
                    |mut parser| {
                        parser
                            .component_registry_mut()
                            .register_component(ComponentRegistration::new(
                                "x-registration-bench",
                                black_box(source.as_str()),
                                None,
                                true,
                            ))
                            .unwrap_or_else(|error| {
                                panic!(
                                    "registration failed for {source_kind} depth {depth}: {error}"
                                )
                            });
                        black_box(parser)
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

criterion_group!(benches, parser_plugin_bench, parser_fast_plugin_bench,);
criterion_main!(benches);
