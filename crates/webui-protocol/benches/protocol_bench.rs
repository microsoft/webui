use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::collections::HashMap;
use std::hint::black_box;
use webui_protocol::{
    ComparisonOperator, ConditionExpr, FragmentList, LogicalOperator, WebUIFragment, WebUIProtocol,
};

#[allow(dead_code)]
fn create_test_protocol() -> WebUIProtocol {
    let mut fragments = HashMap::new();

    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("Hello, WebUI!\n"),
                WebUIFragment::for_loop("person", "people", "for-1"),
                WebUIFragment::signal("description", true),
                WebUIFragment::if_cond(ConditionExpr::identifier("contact"), "if-1"),
            ],
        },
    );

    fragments.insert(
        "for-1".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::signal("person.name", false)],
        },
    );

    fragments.insert(
        "if-1".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::component("contact-card")],
        },
    );

    fragments.insert(
        "contact-card".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("Hello, "),
                WebUIFragment::signal("name", false),
            ],
        },
    );

    WebUIProtocol::new(fragments)
}

fn create_simple_protocol() -> WebUIProtocol {
    let mut fragments = HashMap::new();

    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("Hello, WebUI!\n"),
                WebUIFragment::for_loop("person", "people", "for-1"),
            ],
        },
    );

    fragments.insert(
        "for-1".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::signal("person.name", false)],
        },
    );

    WebUIProtocol::new(fragments)
}

fn serialize_protobuf_benchmark(c: &mut Criterion) {
    let protocol = create_simple_protocol();

    c.bench_function("serialize_protobuf", |b| {
        b.iter(|| black_box(&protocol).to_protobuf())
    });
}

fn deserialize_protobuf_benchmark(c: &mut Criterion) {
    let protocol = create_simple_protocol();
    let bytes = protocol.to_protobuf().expect("encode failed");

    c.bench_function("deserialize_protobuf", |b| {
        b.iter(|| WebUIProtocol::from_protobuf(black_box(&bytes)))
    });
}

fn complex_condition_benchmark(c: &mut Criterion) {
    let nested = ConditionExpr::compound(
        ConditionExpr::predicate("user.role", ComparisonOperator::Equal, "admin"),
        LogicalOperator::And,
        ConditionExpr::negated(ConditionExpr::predicate(
            "user.disabled",
            ComparisonOperator::Equal,
            "true",
        )),
    );

    let mut fragments = HashMap::new();
    fragments.insert(
        "main".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::if_cond(nested, "then")],
        },
    );
    fragments.insert(
        "then".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("ok")],
        },
    );
    let protocol = WebUIProtocol::new(fragments);
    let bytes = protocol.to_protobuf().expect("encode failed");

    c.bench_function("deserialize_complex_condition", |b| {
        b.iter(|| WebUIProtocol::from_protobuf(black_box(&bytes)))
    });
}

fn create_medium_protocol() -> WebUIProtocol {
    let mut fragments = HashMap::new();

    // Root page — head + body structure
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<!DOCTYPE html><html><head><meta charset=\"UTF-8\"><title>"),
                WebUIFragment::signal("title", false),
                WebUIFragment::raw("</title></head><body>"),
                WebUIFragment::component("app"),
                WebUIFragment::raw("<script src=\"/app.js\"></script></body></html>"),
            ],
        },
    );

    // App component — header + list + footer
    fragments.insert(
        "app".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<div class=\"app\"><header><h1>"),
                WebUIFragment::signal("title", false),
                WebUIFragment::raw("</h1><span class=\"count\">"),
                WebUIFragment::signal("remainingCount", false),
                WebUIFragment::raw(" remaining</span></header><div class=\"input-row\"><input type=\"text\" placeholder=\"Add item...\"/><button>Add</button></div><ul class=\"list\">"),
                WebUIFragment::for_loop("item", "items", "item-frag"),
                WebUIFragment::raw("</ul>"),
                WebUIFragment::if_cond(ConditionExpr::identifier("showFooter"), "footer-frag"),
                WebUIFragment::raw("</div>"),
            ],
        },
    );

    // Item fragment — renders each todo item
    fragments.insert(
        "item-frag".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<li"),
                WebUIFragment::attribute("data-id", "item.id"),
                WebUIFragment::attribute_template("class", "item-class-tmpl"),
                WebUIFragment::raw(">"),
                WebUIFragment::signal("item.title", false),
                WebUIFragment::if_cond(
                    ConditionExpr::predicate("item.state", ComparisonOperator::Equal, "'done'"),
                    "done-badge",
                ),
                WebUIFragment::raw(
                    "<button class=\"toggle\">✓</button><button class=\"delete\">✕</button></li>",
                ),
            ],
        },
    );

    // Item class template
    fragments.insert(
        "item-class-tmpl".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("todo-item "),
                WebUIFragment::signal("item.state", false),
            ],
        },
    );

    // Done badge
    fragments.insert(
        "done-badge".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("<span class=\"badge done\">✓</span>")],
        },
    );

    // Footer
    fragments.insert(
        "footer-frag".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<footer><p>"),
                WebUIFragment::signal("footerText", false),
                WebUIFragment::raw("</p><a"),
                WebUIFragment::attribute("href", "helpUrl"),
                WebUIFragment::raw(">Help</a></footer>"),
            ],
        },
    );

    WebUIProtocol::new(fragments)
}

fn create_large_protocol(component_count: usize) -> WebUIProtocol {
    let mut fragments = HashMap::new();

    // Root: nav + main with all components
    let mut root_frags = Vec::with_capacity(component_count * 2 + 4);
    root_frags.push(WebUIFragment::raw("<html><body><nav>"));
    root_frags.push(WebUIFragment::for_loop("link", "navLinks", "nav-link-frag"));
    root_frags.push(WebUIFragment::raw("</nav><main>"));

    for idx in 0..component_count {
        let frag_id = format!("panel-{idx}");
        root_frags.push(WebUIFragment::component(&frag_id));
    }

    root_frags.push(WebUIFragment::raw("</main></body></html>"));

    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: root_frags,
        },
    );

    // Nav link fragment
    fragments.insert(
        "nav-link-frag".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<a"),
                WebUIFragment::attribute("href", "link.url"),
                WebUIFragment::attribute_boolean(
                    "disabled",
                    ConditionExpr::identifier("link.disabled"),
                ),
                WebUIFragment::raw(">"),
                WebUIFragment::signal("link.label", false),
                WebUIFragment::raw("</a>"),
            ],
        },
    );

    // Generate panel components
    for idx in 0..component_count {
        let panel_id = format!("panel-{idx}");
        let body_id = format!("panel-body-{idx}");
        let cond_id = format!("panel-detail-{idx}");

        fragments.insert(
            panel_id,
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw(&format!("<section class=\"panel\" data-idx=\"{idx}\">")),
                    WebUIFragment::raw("<h3>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</h3>"),
                    WebUIFragment::component(&body_id),
                    WebUIFragment::if_cond(ConditionExpr::identifier("showDetails"), &cond_id),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );

        fragments.insert(
            body_id,
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div class=\"panel-body\"><p>"),
                    WebUIFragment::signal("description", false),
                    WebUIFragment::raw("</p><span class=\"metric\">"),
                    WebUIFragment::signal("metric", false),
                    WebUIFragment::raw("</span></div>"),
                ],
            },
        );

        fragments.insert(
            cond_id,
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<details><summary>More</summary><p>"),
                    WebUIFragment::signal("details", false),
                    WebUIFragment::raw("</p></details>"),
                ],
            },
        );
    }

    WebUIProtocol::new(fragments)
}

fn serialize_medium_benchmark(c: &mut Criterion) {
    let protocol = create_medium_protocol();
    c.bench_function("serialize_medium_protobuf", |b| {
        b.iter(|| black_box(&protocol).to_protobuf())
    });
}

fn deserialize_medium_benchmark(c: &mut Criterion) {
    let protocol = create_medium_protocol();
    let bytes = protocol.to_protobuf().expect("encode failed");
    c.bench_function("deserialize_medium_protobuf", |b| {
        b.iter(|| WebUIProtocol::from_protobuf(black_box(&bytes)))
    });
}

fn protocol_size_sweep_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("protocol_size_sweep");

    for &count in &[5usize, 15, 30, 50] {
        let protocol = create_large_protocol(count);
        let bytes = protocol.to_protobuf().expect("encode failed");
        group.throughput(Throughput::Bytes(bytes.len() as u64));

        group.bench_with_input(
            BenchmarkId::new("serialize", count),
            &protocol,
            |b, proto| {
                b.iter(|| black_box(proto).to_protobuf());
            },
        );

        group.bench_with_input(BenchmarkId::new("deserialize", count), &bytes, |b, data| {
            b.iter(|| WebUIProtocol::from_protobuf(black_box(data)));
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    serialize_protobuf_benchmark,
    deserialize_protobuf_benchmark,
    complex_condition_benchmark,
    serialize_medium_benchmark,
    deserialize_medium_benchmark,
    protocol_size_sweep_bench
);
criterion_main!(benches);
