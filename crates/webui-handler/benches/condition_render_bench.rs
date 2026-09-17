// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::hint::black_box;
use webui_handler::{Protocol, RenderOptions, ResponseWriter, WebUIHandler};
use webui_protocol::{
    ComparisonOperator, ConditionExpr, FragmentList, LogicalOperator, WebUIFragment, WebUIProtocol,
};

#[derive(Default)]
struct Writer(String);

impl ResponseWriter for Writer {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.0.push_str(content);
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

fn document() -> WebUIProtocol {
    let condition = ConditionExpr::compound(
        ConditionExpr::predicate("item.status", ComparisonOperator::Equal, "'active'"),
        LogicalOperator::And,
        ConditionExpr::compound(
            ConditionExpr::predicate("item.score", ComparisonOperator::GreaterThanOrEqual, "50"),
            LogicalOperator::And,
            ConditionExpr::negated(ConditionExpr::identifier("item.hidden")),
        ),
    );
    let records = [
        (
            "index.html",
            vec![
                WebUIFragment::raw("<ul>"),
                WebUIFragment::for_loop("item", "items", "row"),
                WebUIFragment::raw("</ul>"),
            ],
        ),
        ("row", vec![WebUIFragment::if_cond(condition, "visible")]),
        (
            "visible",
            vec![
                WebUIFragment::raw("<li><button"),
                WebUIFragment::attribute_boolean(
                    "disabled",
                    ConditionExpr::negated(ConditionExpr::identifier("item.enabled")),
                ),
                WebUIFragment::raw(">"),
                WebUIFragment::signal("item.name", false),
                WebUIFragment::raw("</button></li>"),
            ],
        ),
    ];
    let fragments: HashMap<_, _> = records
        .into_iter()
        .map(|(id, fragments)| {
            (
                id.to_string(),
                FragmentList {
                    fragments,
                    contains_boundary: false,
                },
            )
        })
        .collect();
    WebUIProtocol::new(fragments)
}

fn state(count: usize, mixed: bool) -> (Value, String) {
    let mut items = Vec::with_capacity(count);
    let mut expected = String::with_capacity(count * 48 + 9);
    expected.push_str("<ul>");
    for index in 0..count {
        let active = !mixed || index % 3 != 0;
        let score = if mixed { index % 100 } else { 75 };
        let hidden = mixed && index % 7 == 0;
        let enabled = index % 2 == 0;
        let name = format!("Item {index}");
        if active && score >= 50 && !hidden {
            expected.push_str(if enabled {
                "<li><button>"
            } else {
                "<li><button disabled>"
            });
            expected.push_str(&name);
            expected.push_str("</button></li>");
        }
        items.push(json!({
            "status": if active { "active" } else { "inactive" },
            "score": score,
            "hidden": hidden,
            "enabled": enabled,
            "name": name,
        }));
    }
    expected.push_str("</ul>");
    (json!({ "items": items }), expected)
}

fn condition_render(c: &mut Criterion) {
    let mut group = c.benchmark_group("condition_render");
    let protocol = Protocol::new(document());
    let handler = WebUIHandler::new();
    let options = RenderOptions::new("index.html", "/");
    for mixed in [false, true] {
        for count in [10, 100, 1000] {
            let (state, expected) = state(count, mixed);
            let mut writer = Writer(String::with_capacity(expected.len()));
            handler
                .render(&protocol, &state, &options, &mut writer)
                .unwrap_or_else(|error| panic!("condition render failed: {error}"));
            assert_eq!(writer.0, expected);
            group.throughput(Throughput::Bytes(
                u64::try_from(expected.len()).unwrap_or_else(|_| panic!("output too large")),
            ));
            let case = if mixed { "mixed" } else { "all_visible" };
            group.bench_with_input(BenchmarkId::new(case, count), &state, |b, state| {
                b.iter(|| {
                    writer.0.clear();
                    handler
                        .render(
                            black_box(&protocol),
                            black_box(state),
                            &options,
                            &mut writer,
                        )
                        .unwrap_or_else(|error| panic!("condition render failed: {error}"));
                    black_box(&writer.0);
                });
            });
        }
    }
    group.finish();
}

fn condition_protocol_load(c: &mut Criterion) {
    let document = document();
    c.bench_function("condition_protocol_load", |b| {
        b.iter_batched(
            || document.clone(),
            |document| Protocol::new(black_box(document)),
            criterion::BatchSize::SmallInput,
        );
    });
}

criterion_group!(benches, condition_render, condition_protocol_load);
criterion_main!(benches);
