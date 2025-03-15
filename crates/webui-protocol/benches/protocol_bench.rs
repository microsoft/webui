use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::collections::HashMap;
use webui_protocol::{
    ConditionExpr, LogicalOperator, Predicate, WebUIProtocol, WebUIStream,
    WebUIStreamComponent, WebUIStreamFor, WebUIStreamIf, WebUIStreamRaw, WebUIStreamSignal,
    ComparisonOperator,
};

fn create_test_protocol() -> WebUIProtocol {
    let mut streams = HashMap::new();
    
    // Create the protocol structure directly in Rust
    streams.insert(
        "index.html".to_string(),
        vec![
            WebUIStream::Raw(WebUIStreamRaw {
                value: "Hello, WebUI!\n".to_string(),
            }),
            WebUIStream::For(WebUIStreamFor {
                item: "person".to_string(),
                collection: "people".to_string(),
                stream_id: "for-1".to_string(),
            }),
            WebUIStream::Signal(WebUIStreamSignal {
                value: "description".to_string(),
                raw: true,
            }),
            WebUIStream::If(WebUIStreamIf {
                condition: ConditionExpr::Identifier {
                    value: "contact".to_string(),
                },
                stream_id: "if-1".to_string(),
            }),
        ],
    );
    
    streams.insert(
        "for-1".to_string(),
        vec![
            WebUIStream::Signal(WebUIStreamSignal {
                value: "person.name".to_string(),
                raw: false,
            }),
        ],
    );
    
    streams.insert(
        "if-1".to_string(),
        vec![
            WebUIStream::Component(WebUIStreamComponent {
                css: "".to_string(),
                stream_id: "contact-card".to_string(),
            }),
        ],
    );
    
    streams.insert(
        "contact-card".to_string(),
        vec![
            WebUIStream::Raw(WebUIStreamRaw {
                value: "Hello, ".to_string(),
            }),
            WebUIStream::Signal(WebUIStreamSignal {
                value: "name".to_string(),
                raw: false,
            }),
        ],
    );
    
    WebUIProtocol { streams }
}

fn create_simple_protocol() -> WebUIProtocol {
    let mut streams = HashMap::new();
    
    streams.insert(
        "index.html".to_string(),
        vec![
            WebUIStream::Raw(WebUIStreamRaw {
                value: "Hello, WebUI!\n".to_string(),
            }),
            WebUIStream::For(WebUIStreamFor {
                item: "person".to_string(),
                collection: "people".to_string(),
                stream_id: "for-1".to_string(),
            }),
        ],
    );
    
    streams.insert(
        "for-1".to_string(),
        vec![
            WebUIStream::Signal(WebUIStreamSignal {
                value: "person.name".to_string(),
                raw: false,
            }),
        ],
    );
    
    WebUIProtocol { streams }
}

fn serialize_json_benchmark(c: &mut Criterion) {
    let protocol = create_simple_protocol();
    
    c.bench_function("serialize_json", |b| {
        b.iter(|| black_box(&protocol).to_json())
    });
}

fn complex_condition_benchmark(c: &mut Criterion) {
    // Create a complex nested condition
    let nested = ConditionExpr::Compound {
        left: Box::new(ConditionExpr::Predicate(Predicate {
            left: "user.role".to_string(),
            operator: ComparisonOperator::Equal,
            right: "admin".to_string(),
        })),
        op: LogicalOperator::And,
        right: Box::new(ConditionExpr::Not(Box::new(ConditionExpr::Predicate(Predicate {
            left: "user.disabled".to_string(),
            operator: ComparisonOperator::Equal,
            right: "true".to_string(),
        })))),
    };
    
    c.bench_function("serialize_complex_condition", |b| {
        b.iter(|| serde_json::to_string(black_box(&nested)))
    });
}

criterion_group!(
    benches, 
    serialize_json_benchmark,
    complex_condition_benchmark
);
criterion_main!(benches);
