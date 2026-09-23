// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::HashMap;
use std::hint::black_box;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use serde_json::{json, Value};
use webui_handler::Protocol;
use webui_protocol::{
    FragmentList, StateProjectionMode, WebUIFragment, WebUIProtocol, WebUiFragmentRoute,
};

// Isolate the current navigation serialization boundaries, not native startup or
// end-to-end page readiness. Owned inputs and serialized raw inputs are prepared
// outside timing. The historical clone/render/reparse/replace pipeline is not a
// production baseline and is deliberately not included.
fn protocol() -> Protocol {
    Protocol::new(protocol_model())
}

fn protocol_model() -> WebUIProtocol {
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
            navigation_mode: Some(StateProjectionMode::Keys as i32),
            navigation_keys: vec!["basePath".into(), "contacts".into(), "title".into()],
            ..Default::default()
        },
    );
    protocol
}

fn desktop_pipeline(
    protocol: &Protocol,
    state: Value,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let partial = protocol.prepare_partial(state, "index.html", "/contacts", "")?;
    assert!(partial.is_match());
    Ok(serde_json::to_vec(&partial)?)
}

fn web_owned_pipeline(
    protocol: &Protocol,
    state: Value,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    Ok(protocol
        .render_partial(state, "index.html", "/contacts", "")?
        .into_bytes())
}

fn web_raw_pipeline(
    protocol: &Protocol,
    state: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    Ok(protocol
        .render_partial_json(state, "index.html", "/contacts", "")?
        .into_bytes())
}

fn checked<T>(result: Result<T, impl std::fmt::Display>) -> T {
    match result {
        Ok(bytes) => bytes,
        Err(error) => panic!("navigation benchmark fixture failed: {error}"),
    }
}

// serde_json's fixture-only json! expansion uses unwrap for infallible primitives.
#[allow(clippy::disallowed_methods)]
fn route_state(rows: usize, unused_fields: bool) -> Value {
    let contacts: Vec<_> = (0..rows)
        .map(|id| {
            json!({
            "id": id, "name": format!("Contact {id}"), "email": format!("contact{id}@example.test"),
            "company": "Example company", "favorite": id % 3 == 0,
            "tags": ["customer", "newsletter"], "notes": "A route-owned contact view model.",
            })
        })
        .collect();
    let mut state = json!({"title": "Contacts", "basePath": "/", "contacts": contacts});
    if unused_fields {
        // Data for another route: four reports per required contact. Several
        // owned fields per report exercise both traversal and input destruction,
        // rather than modeling all unused data as one allocation.
        let reports: Vec<_> = (0..rows * 4)
            .map(|id| {
                json!({
                    "id": id,
                    "title": format!("Report {id}"),
                    "body": "Archive detail. ".repeat(16),
                })
            })
            .collect();
        state["unusedReports"] = Value::Array(reports);
    }
    state
}

fn checked_response(protocol: &Protocol, state: &Value, state_json: &str) -> Vec<u8> {
    let desktop = checked(desktop_pipeline(protocol, state.clone()));
    let owned = checked(web_owned_pipeline(protocol, state.clone()));
    let raw = checked(web_raw_pipeline(protocol, state_json));
    assert_eq!(
        desktop, owned,
        "prepared desktop and owned web responses must be byte-identical"
    );
    assert_eq!(
        desktop, raw,
        "normalized raw input must produce identical response bytes"
    );
    desktop
}

#[allow(clippy::disallowed_methods)]
fn check_fixture(protocol: &Protocol, state: &Value, state_json: &str, rows: usize) {
    let bytes = checked_response(protocol, state, state_json);
    let response: Value = checked(serde_json::from_slice(&bytes));
    assert_eq!(response["state"], route_state(rows, false));
    assert_eq!(response["path"], "/contacts");
    assert_eq!(
        response["chain"],
        json!([{"component": "contact-list", "path": "/contacts", "exact": true}])
    );
    assert_eq!(response["inventory"], "01");
    assert_eq!(
        response["templates"]["contact-list"],
        json!({"h": "<main>Contacts</main>", "th": 1})
    );
    eprintln!(
        "navigation fixture: {rows} contacts, input={} B, {} identical response bytes",
        state_json.len(),
        bytes.len()
    );
}

#[allow(clippy::disallowed_methods)]
fn check_reserved_state() {
    let mut state = route_state(1, true);
    state["$webui"] = json!({"bodyEnd": "<script>server-only</script>"});
    let raw_state = checked(serde_json::to_string(&state));
    for mode in [
        Some(StateProjectionMode::Keys as i32),
        Some(StateProjectionMode::All as i32),
        None,
    ] {
        let mut model = protocol_model();
        let component = checked(
            model
                .components
                .get_mut("contact-list")
                .ok_or("missing fixture component"),
        );
        component.navigation_mode = mode;
        let projected = mode == Some(StateProjectionMode::Keys as i32);
        if projected {
            // Even an explicitly requested reserved root must stay off the wire.
            component.navigation_keys.insert(0, "$webui".into());
        } else {
            component.navigation_keys.clear();
        }
        let response = checked_response(&Protocol::new(model), &state, &raw_state);
        let response: Value = checked(serde_json::from_slice(&response));
        assert!(response["state"].get("$webui").is_none());
        assert_eq!(response["state"], route_state(1, !projected));
    }
}

fn navigation(c: &mut Criterion) {
    let protocol = protocol();
    check_reserved_state();
    let mut group = c.benchmark_group("shared_navigation");
    group.sample_size(30);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(2));
    for rows in [25, 1000] {
        for (shape, unused_fields) in [("all_required", false), ("unused_fields", true)] {
            let state = route_state(rows, unused_fields);
            let state_json = checked(serde_json::to_string(&state));
            check_fixture(&protocol, &state, &state_json, rows);
            for (name, pipeline) in [
                (
                    "desktop_owned",
                    desktop_pipeline
                        as fn(&Protocol, Value) -> Result<Vec<u8>, Box<dyn std::error::Error>>,
                ),
                ("web_owned", web_owned_pipeline),
            ] {
                group.bench_function(BenchmarkId::new(format!("{shape}/{name}"), rows), |b| {
                    b.iter_batched(
                        || state.clone(),
                        |state| {
                            black_box(checked(pipeline(&protocol, state)));
                        },
                        // Bound setup memory to one state tree. Large batches of
                        // the unused-fields fixture distort allocator/cache costs.
                        BatchSize::PerIteration,
                    );
                });
            }
            group.bench_function(BenchmarkId::new(format!("{shape}/web_raw"), rows), |b| {
                b.iter(|| {
                    black_box(checked(web_raw_pipeline(&protocol, &state_json)));
                });
            });
        }
    }
    group.finish();
}

criterion_group!(benches, navigation);
criterion_main!(benches);
