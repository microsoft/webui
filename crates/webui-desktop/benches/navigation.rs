// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::HashMap;
use std::hint::black_box;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use serde_json::{json, Value};
use webui_handler::Protocol;
use webui_protocol::{FragmentList, WebUIFragment, WebUIProtocol, WebUiFragmentRoute};

// Isolate the desktop navigation serialization boundary, not native startup or
// end-to-end page readiness. Both cases receive the same owned route view model.
fn protocol() -> Protocol {
    let mut protocol = WebUIProtocol::new(HashMap::from([
        (
            "index.html".into(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/contacts".into(),
                    fragment_id: "contact-list".into(),
                    exact: true,
                    ..Default::default()
                })],
                ..Default::default()
            },
        ),
        (
            "contact-list".into(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<main>Contacts</main>")],
                ..Default::default()
            },
        ),
    ]));
    protocol.components.insert(
        "contact-list".into(),
        webui_protocol::ComponentData {
            template_json: r#"{"h":"<main>Contacts</main>","th":1}"#.into(),
            navigation_mode: Some(webui_protocol::StateProjectionMode::Keys as i32),
            navigation_keys: vec!["contacts".into(), "title".into(), "basePath".into()],
            ..Default::default()
        },
    );
    Protocol::new(protocol)
}

fn previous_pipeline(
    protocol: &Protocol,
    state: Value,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let json = protocol.render_partial(state.clone(), "index.html", "/contacts", "")?;
    let mut partial: Value = serde_json::from_str(&json)?;
    partial["state"] = state;
    Ok(serde_json::to_vec(&partial)?)
}

fn typed_pipeline(
    protocol: &Protocol,
    state: Value,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let partial = protocol.render_partial_full_state(state, "index.html", "/contacts", "")?;
    assert!(partial.is_match());
    Ok(serde_json::to_vec(&partial)?)
}

fn checked(result: Result<Vec<u8>, Box<dyn std::error::Error>>) -> Vec<u8> {
    match result {
        Ok(bytes) => bytes,
        Err(error) => panic!("navigation benchmark fixture failed: {error}"),
    }
}

// serde_json's fixture-only json! expansion uses unwrap for infallible primitives.
#[allow(clippy::disallowed_methods)]
fn navigation(c: &mut Criterion) {
    let protocol = protocol();
    let mut group = c.benchmark_group("desktop_navigation");
    group.sample_size(30);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(2));
    for rows in [25, 1000] {
        let contacts: Vec<_> = (0..rows).map(|id| json!({
            "id": id, "name": format!("Contact {id}"), "email": format!("contact{id}@example.test"),
            "company": "Example company", "favorite": id % 3 == 0,
            "tags": ["customer", "newsletter"], "notes": "A route-owned contact view model.",
        })).collect();
        let state = json!({"title": "Contacts", "basePath": "/", "contacts": contacts});
        let expected = checked(previous_pipeline(&protocol, state.clone()));
        assert_eq!(expected, checked(typed_pipeline(&protocol, state.clone())));
        eprintln!(
            "navigation fixture: {rows} contacts, {} identical response bytes",
            expected.len()
        );
        for (name, pipeline) in [
            (
                "serialize_reparse",
                previous_pipeline
                    as fn(&Protocol, Value) -> Result<Vec<u8>, Box<dyn std::error::Error>>,
            ),
            ("typed", typed_pipeline),
        ] {
            group.bench_function(BenchmarkId::new(name, rows), |b| {
                b.iter_batched(
                    || state.clone(),
                    |state| {
                        black_box(checked(pipeline(&protocol, state)));
                    },
                    BatchSize::SmallInput,
                );
            });
        }
    }
    group.finish();
}

criterion_group!(benches, navigation);
criterion_main!(benches);
