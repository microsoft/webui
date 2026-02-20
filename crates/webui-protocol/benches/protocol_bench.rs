use criterion::{criterion_group, criterion_main, Criterion};
use std::collections::HashMap;
use std::hint::black_box;
use webui_protocol::{
    ComparisonOperator, ConditionExpr, LogicalOperator, Predicate, WebUIFragment,
    WebUIFragmentComponent, WebUIFragmentFor, WebUIFragmentIf, WebUIFragmentRaw,
    WebUIFragmentSignal, WebUIProtocol,
};

#[allow(dead_code)]
fn create_test_protocol() -> WebUIProtocol {
    let mut fragments = HashMap::new();

    fragments.insert(
        "index.html".to_string(),
        vec![
            WebUIFragment::Raw(WebUIFragmentRaw {
                value: "Hello, WebUI!\n".to_string(),
            }),
            WebUIFragment::For(WebUIFragmentFor {
                item: "person".to_string(),
                collection: "people".to_string(),
                fragment_id: "for-1".to_string(),
            }),
            WebUIFragment::Signal(WebUIFragmentSignal {
                value: "description".to_string(),
                raw: true,
            }),
            WebUIFragment::If(WebUIFragmentIf {
                condition: ConditionExpr::Identifier {
                    value: "contact".to_string(),
                },
                fragment_id: "if-1".to_string(),
            }),
        ],
    );

    fragments.insert(
        "for-1".to_string(),
        vec![WebUIFragment::Signal(WebUIFragmentSignal {
            value: "person.name".to_string(),
            raw: false,
        })],
    );

    fragments.insert(
        "if-1".to_string(),
        vec![WebUIFragment::Component(WebUIFragmentComponent {
            fragment_id: "contact-card".to_string(),
        })],
    );

    fragments.insert(
        "contact-card".to_string(),
        vec![
            WebUIFragment::Raw(WebUIFragmentRaw {
                value: "Hello, ".to_string(),
            }),
            WebUIFragment::Signal(WebUIFragmentSignal {
                value: "name".to_string(),
                raw: false,
            }),
        ],
    );

    WebUIProtocol { fragments }
}

fn create_simple_protocol() -> WebUIProtocol {
    let mut fragments = HashMap::new();

    fragments.insert(
        "index.html".to_string(),
        vec![
            WebUIFragment::Raw(WebUIFragmentRaw {
                value: "Hello, WebUI!\n".to_string(),
            }),
            WebUIFragment::For(WebUIFragmentFor {
                item: "person".to_string(),
                collection: "people".to_string(),
                fragment_id: "for-1".to_string(),
            }),
        ],
    );

    fragments.insert(
        "for-1".to_string(),
        vec![WebUIFragment::Signal(WebUIFragmentSignal {
            value: "person.name".to_string(),
            raw: false,
        })],
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
    let nested = ConditionExpr::Compound {
        left: Box::new(ConditionExpr::Predicate(Predicate {
            left: "user.role".to_string(),
            operator: ComparisonOperator::Equal,
            right: "admin".to_string(),
        })),
        op: LogicalOperator::And,
        right: Box::new(ConditionExpr::Not(Box::new(ConditionExpr::Predicate(
            Predicate {
                left: "user.disabled".to_string(),
                operator: ComparisonOperator::Equal,
                right: "true".to_string(),
            },
        )))),
    };

    let mut fragments = HashMap::new();
    fragments.insert(
        "main".to_string(),
        vec![WebUIFragment::If(WebUIFragmentIf {
            condition: nested,
            fragment_id: "then".to_string(),
        })],
    );
    fragments.insert(
        "then".to_string(),
        vec![WebUIFragment::Raw(WebUIFragmentRaw {
            value: "ok".to_string(),
        })],
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
