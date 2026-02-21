use criterion::{criterion_group, criterion_main, Criterion};
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

    WebUIProtocol { fragments }
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

    WebUIProtocol { fragments }
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
    let protocol = WebUIProtocol { fragments };
    let bytes = protocol.to_protobuf().expect("encode failed");

    c.bench_function("deserialize_complex_condition", |b| {
        b.iter(|| WebUIProtocol::from_protobuf(black_box(&bytes)))
    });
}

criterion_group!(
    benches,
    serialize_protobuf_benchmark,
    deserialize_protobuf_benchmark,
    complex_condition_benchmark
);
criterion_main!(benches);
