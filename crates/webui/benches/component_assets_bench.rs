// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Static component asset graph rendering benchmarks.
//!
//! Run with:
//! `cargo bench -p microsoft-webui --bench component_assets_bench`

use criterion::{criterion_group, BenchmarkId, Criterion};
use std::collections::HashMap;
use std::hint::black_box;
use std::time::Duration;
use webui::render_component_assets;
use webui_protocol::{FragmentList, WebUIFragment, WebUIProtocol};

const ROOT_COUNT: usize = 12;
const SHARED_COMPONENT_COUNT: usize = 32;
const TEMPLATE_TEXT_BYTES: usize = 512;

struct Fixture {
    protocol: WebUIProtocol,
    single_root: Vec<String>,
    overlapping_roots: Vec<String>,
}

fn component_tag(prefix: &str, index: usize) -> String {
    format!("{prefix}-{index}")
}

fn template_json(tag: &str) -> String {
    let body = "x".repeat(TEMPLATE_TEXT_BYTES);
    format!(r#"{{"h":"<section data-tag=\"{tag}\">{body}</section>"}}"#)
}

fn setup() -> Fixture {
    let mut fragments = HashMap::new();
    let mut roots = Vec::with_capacity(ROOT_COUNT);

    for root_index in 0..ROOT_COUNT {
        let root = component_tag("asset-root", root_index);
        let unique = component_tag("unique-child", root_index);
        let mut root_fragments = Vec::with_capacity(SHARED_COMPONENT_COUNT + 1);
        for shared_index in 0..SHARED_COMPONENT_COUNT {
            root_fragments.push(WebUIFragment::component(component_tag(
                "shared-child",
                shared_index,
            )));
        }
        root_fragments.push(WebUIFragment::component(unique.clone()));
        fragments.insert(
            root.clone(),
            FragmentList {
                fragments: root_fragments,
                contains_boundary: false,
            },
        );
        fragments.insert(
            unique,
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>unique</p>")],
                contains_boundary: false,
            },
        );
        roots.push(root);
    }

    for shared_index in 0..SHARED_COMPONENT_COUNT {
        fragments.insert(
            component_tag("shared-child", shared_index),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>shared</p>")],
                contains_boundary: false,
            },
        );
    }

    let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
    protocol.fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("<main></main>")],
            contains_boundary: false,
        },
    );
    let tags: Vec<String> = protocol.fragments.keys().cloned().collect();
    for tag in tags {
        protocol
            .components
            .entry(tag.clone())
            .or_default()
            .template_json = template_json(&tag);
    }
    protocol.populate_style_closures(&["index.html"]);

    Fixture {
        protocol,
        single_root: vec![roots[0].clone()],
        overlapping_roots: roots,
    }
}

fn render(protocol: &WebUIProtocol, roots: &[String]) -> Vec<webui::ComponentAssetFile> {
    match render_component_assets(protocol, "index.html", roots, "[name].[ext]", false) {
        Ok(graph) => graph.files,
        Err(error) => panic!("component asset benchmark setup failed: {error}"),
    }
}

fn output_bytes(files: &[webui::ComponentAssetFile]) -> usize {
    files.iter().map(|file| file.content.len()).sum()
}

fn component_assets_bench(c: &mut Criterion) {
    let fixture = setup();
    let mut group = c.benchmark_group("component_assets");
    group.measurement_time(Duration::from_secs(8));
    group.sample_size(50);

    for (name, roots) in [
        ("single-root", fixture.single_root.as_slice()),
        ("overlap-12", fixture.overlapping_roots.as_slice()),
    ] {
        group.bench_with_input(BenchmarkId::from_parameter(name), roots, |b, roots| {
            b.iter(|| {
                let files = render(black_box(&fixture.protocol), black_box(roots));
                black_box(files);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, component_assets_bench);

fn main() {
    let fixture = setup();
    let single = render(&fixture.protocol, &fixture.single_root);
    let overlap = render(&fixture.protocol, &fixture.overlapping_roots);
    eprintln!(
        "component-assets baseline: single files={} bytes={}; overlap files={} bytes={} largest={}",
        single.len(),
        output_bytes(&single),
        overlap.len(),
        output_bytes(&overlap),
        overlap
            .iter()
            .map(|file| file.content.len())
            .max()
            .unwrap_or(0)
    );
    benches();
}
