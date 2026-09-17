// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::hint::black_box;
use webui_handler::{
    ConditionEvaluation, Protocol, ProtocolOptions, RenderOptions, ResponseWriter, WebUIHandler,
};
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
    for (name, mode) in [
        ("condition_render", ConditionEvaluation::Prepared),
        ("condition_render_direct", ConditionEvaluation::Direct),
    ] {
        condition_render_mode(c, name, mode);
    }
}

fn condition_render_mode(c: &mut Criterion, name: &str, mode: ConditionEvaluation) {
    let mut group = c.benchmark_group(name);
    let protocol = Protocol::new_with_options(
        document(),
        ProtocolOptions {
            condition_evaluation: mode,
        },
    );
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
    for (name, mode) in [
        ("condition_protocol_load", ConditionEvaluation::Prepared),
        (
            "condition_protocol_load_direct",
            ConditionEvaluation::Direct,
        ),
    ] {
        c.bench_function(name, |b| {
            b.iter_batched(
                || document.clone(),
                |document| {
                    Protocol::new_with_options(
                        black_box(document),
                        ProtocolOptions {
                            condition_evaluation: mode,
                        },
                    )
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }
}

#[derive(Clone, Copy)]
enum ConditionKind {
    Identifier,
    StringPredicate,
    Compound,
}

impl ConditionKind {
    fn name(self) -> &'static str {
        match self {
            Self::Identifier => "identifier",
            Self::StringPredicate => "string_predicate",
            Self::Compound => "compound",
        }
    }

    fn expression(self, path: &str) -> ConditionExpr {
        match self {
            Self::Identifier => ConditionExpr::identifier(format!("{path}.enabled")),
            Self::StringPredicate => ConditionExpr::predicate(
                format!("{path}.status"),
                ComparisonOperator::Equal,
                "'active'",
            ),
            Self::Compound => ConditionExpr::compound(
                ConditionExpr::predicate(
                    format!("{path}.status"),
                    ComparisonOperator::Equal,
                    "'active'",
                ),
                LogicalOperator::And,
                ConditionExpr::negated(ConditionExpr::identifier(format!("{path}.hidden"))),
            ),
        }
    }
}

fn distinct_document(kind: ConditionKind) -> (WebUIProtocol, Value) {
    let mut conditions = Vec::with_capacity(1000);
    let mut state = serde_json::Map::new();
    for index in 0..1000 {
        let path = format!("item_{index:04}");
        conditions.push(WebUIFragment::if_cond(kind.expression(&path), "selected"));
        state.insert(
            path,
            json!({ "enabled": true, "status": "active", "hidden": false }),
        );
    }
    let records = HashMap::from([
        (
            "index.html".to_string(),
            FragmentList {
                fragments: conditions,
                contains_boundary: false,
            },
        ),
        (
            "selected".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("x")],
                contains_boundary: false,
            },
        ),
    ]);
    (WebUIProtocol::new(records), Value::Object(state))
}

fn distinct_condition_render(c: &mut Criterion) {
    let mut group = c.benchmark_group("distinct_conditions_1000");
    let handler = WebUIHandler::new();
    let options = RenderOptions::new("index.html", "/");
    let expected = "x".repeat(1000);
    group.throughput(Throughput::Elements(1000));
    for kind in [
        ConditionKind::Identifier,
        ConditionKind::StringPredicate,
        ConditionKind::Compound,
    ] {
        let (document, state) = distinct_document(kind);
        for (label, mode) in [
            ("prepared", ConditionEvaluation::Prepared),
            ("direct", ConditionEvaluation::Direct),
        ] {
            let protocol = Protocol::new_with_options(
                document.clone(),
                ProtocolOptions {
                    condition_evaluation: mode,
                },
            );
            let mut writer = Writer(String::with_capacity(expected.len()));
            handler
                .render(&protocol, &state, &options, &mut writer)
                .unwrap_or_else(|error| panic!("distinct condition render failed: {error}"));
            assert_eq!(writer.0, expected);
            group.bench_function(BenchmarkId::new(kind.name(), label), |b| {
                b.iter(|| {
                    writer.0.clear();
                    handler
                        .render(
                            black_box(&protocol),
                            black_box(&state),
                            &options,
                            &mut writer,
                        )
                        .unwrap_or_else(|error| {
                            panic!("distinct condition render failed: {error}")
                        });
                    black_box(&writer.0);
                });
            });
        }
    }
    group.finish();
}

criterion_group!(
    benches,
    condition_render,
    condition_protocol_load,
    distinct_condition_render
);
criterion_main!(benches);
