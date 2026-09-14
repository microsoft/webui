// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(
    clippy::disallowed_methods,
    reason = "serde_json::json! internally unwraps infallible fixture values"
)]

mod build_control_support;
#[path = "build_control_support/dev.rs"]
mod dev;
#[path = "build_control_support/diagnostics.rs"]
mod diagnostics;
#[path = "build_control_support/lifecycle.rs"]
mod lifecycle;
#[path = "build_control_support/scheduling.rs"]
mod scheduling;
#[cfg(windows)]
#[path = "build_control_support/windows_console.rs"]
mod windows_console;

use std::fs;
use std::time::Duration;

use anyhow::Result;
use serde_json::json;

use build_control_support::{
    assert_gate, bootstrap, document_nonce, request, Backend, Events, Fixture, Server,
};

#[test]
fn native_client_builds_gate_responses_reload_inputs_and_recover() -> Result<()> {
    let fixture = Fixture::new()?;
    fixture.configure(json!({ "title": "build-one", "dark": true }))?;
    assert!(!fixture.dist.join("theme.json").exists());
    assert!(!fixture.dist.join("state.json").exists());
    let mut server = Server::spawn(fixture.command(0))?;
    let address = server.listening()?;
    let mut events = Events::connect(address)?;
    assert_gate(address, 503)?;
    events.quiet()?;
    events.expect("reload")?;
    let first = request(address, "/", "text/html")?;
    assert_eq!(first.status, 200);
    assert!(first.body.contains("build-one"));
    assert!(first.body.contains("--brand: #112233;"));
    assert!(first.body.contains("--brand: #abcdef;"));
    assert!(first.body.contains("unrelated-token-value"));
    assert!(!first.body.contains("must-be-replaced"));
    let state = bootstrap(&first.body)?;
    assert_eq!(state["state"]["title"], "build-one");
    assert_eq!(state["state"]["appValue"], "preserved");
    assert_eq!(request(address, "/client.js", "*/*")?.status, 200);
    assert_eq!(request(address, "/test-card.css", "*/*")?.status, 200);
    assert_eq!(
        request(address, "/_webui/templates?t=test-card", "application/json")?.status,
        200
    );
    events.quiet()?;

    fixture.configure(json!({
        "title": "failed-client-output", "brand": "#445566", "fail": true
    }))?;
    fixture.wait_event("begin", "failed-client-output")?;
    assert_gate(address, 503)?;
    events.expect("reload-error")?;
    fixture.wait_event("written", "failed-client-output")?;
    assert!(fs::read_to_string(fixture.dist.join("client.js"))?.contains("failed-client-output"));
    assert_gate(address, 500)?;
    events.quiet()?;
    assert!(server
        .logs()
        .contains("Client plugin failed after emitting files"));
    assert!(server.logs().contains("café"));

    fixture.configure(json!({
        "title": "build-three", "brand": "#778899", "delayMs": 300
    }))?;
    events.expect("reload")?;
    let third = request(address, "/", "text/html")?;
    assert_eq!(third.status, 200);
    assert!(third.body.contains("build-three"));
    assert!(third.body.contains("--brand: #778899;"));
    assert!(!third.body.contains("--brand: #112233;"));
    assert!(!third.body.contains("--brand: #abcdef;"));
    assert!(!third.body.contains("build-one"));
    assert!(!third.body.contains("failed-client-output"));
    assert!(third.body.contains("unrelated-token-value"));
    assert_eq!(bootstrap(&third.body)?["state"]["title"], "build-three");

    // These outlive the debounce window and include atomic writer scratch files.
    fs::write(fixture.dist.join("bundle-stage.tmp"), b"stage")?;
    fs::rename(
        fixture.dist.join("bundle-stage.tmp"),
        fixture.dist.join("bundle-renamed.js"),
    )?;
    fs::remove_file(fixture.dist.join("bundle-renamed.js"))?;
    server.assert_alive(Duration::from_millis(800))?;
    events.quiet()?;
    assert_eq!(fixture.count("begin")?, 3, "generated outputs are ignored");
    fixture.assert_serial()?;
    server.shutdown()?;
    assert_eq!(fixture.count("dispose-begin")?, 1);
    assert_eq!(fixture.count("dispose-end")?, 1);
    Ok(())
}

#[test]
fn native_missing_and_malformed_generated_inputs_fail_without_stale_success() -> Result<()> {
    let fixture = Fixture::new()?;
    fixture.configure(json!({ "mode": "missing" }))?;
    let mut server = Server::spawn(fixture.command(0))?;
    let address = server.listening()?;
    let mut events = Events::connect(address)?;
    for mode in ["missing", "bad-state", "bad-theme", "missing-token"] {
        if mode != "missing" {
            fixture.configure(json!({ "title": mode, "mode": mode, "delayMs": 300 }))?;
        }
        events.expect("reload-error")?;
        assert_gate(address, 500)?;
        events.quiet()?;
        server.assert_alive(Duration::from_millis(30))?;
    }
    fixture.configure(json!({
        "title": "recovered-inputs", "brand": "#445566", "delayMs": 300
    }))?;
    events.expect("reload")?;
    let response = request(address, "/", "text/html")?;
    assert_eq!(response.status, 200);
    assert!(response.body.contains("recovered-inputs"));
    assert!(response.body.contains("--brand: #445566;"));
    events.quiet()?;
    fixture.assert_serial()?;
    assert_eq!(fixture.count("begin")?, 5);
    server.shutdown()
}

#[test]
fn native_headers_and_csp_use_fresh_matching_sdk_nonces() -> Result<()> {
    let fixture = Fixture::new()?;
    fixture.configure(json!({ "title": "nonce-build" }))?;
    let mut command = fixture.command(0);
    command.args([
        "--header",
        "X-Test: native: value",
        "--csp",
        "script-src 'self' 'nonce-{nonce}'; style-src 'nonce-{nonce}'",
    ]);
    let mut server = Server::spawn(command)?;
    let address = server.listening()?;
    let mut events = Events::connect(address)?;
    assert!(events.headers.contains("x-test: native: value"));
    assert!(!events.headers.contains("content-security-policy:"));
    let pending = request(address, "/", "text/html")?;
    assert_eq!(pending.status, 503);
    assert_eq!(pending.header("x-test"), "native: value");
    let pending_nonce = document_nonce(&pending)?;

    events.expect("reload")?;
    let first = request(address, "/", "text/html")?;
    let second = request(address, "/", "text/html")?;
    assert_eq!(first.status, 200);
    assert_eq!(second.status, 200);
    let first_nonce = document_nonce(&first)?;
    let second_nonce = document_nonce(&second)?;
    assert_ne!(pending_nonce, first_nonce);
    assert_ne!(
        first_nonce, second_nonce,
        "HTML cache must not reuse nonces"
    );
    assert_eq!(bootstrap(&first.body)?["nonce"], first_nonce);
    assert_eq!(bootstrap(&second.body)?["nonce"], second_nonce);
    assert_eq!(first.header("x-test"), "native: value");

    for (path, accept) in [
        ("/client.js", "*/*"),
        ("/test-card.css", "*/*"),
        ("/missing.png", "image/png"),
        ("/", "application/json"),
        ("/_webui/templates?t=test-card", "application/json"),
    ] {
        let response = request(address, path, accept)?;
        assert_eq!(response.header("x-test"), "native: value", "{path}");
        assert_eq!(response.header("content-security-policy"), "", "{path}");
    }
    fixture.configure(json!({ "title": "failed", "fail": true, "delayMs": 300 }))?;
    events.expect("reload-error")?;
    let failed = request(address, "/", "text/html")?;
    assert_eq!(failed.status, 500);
    assert_eq!(failed.header("x-test"), "native: value");
    assert_ne!(document_nonce(&failed)?, second_nonce);
    events.quiet()?;
    server.shutdown()
}

#[test]
fn native_strict_api_acquires_fresh_state_and_never_falls_back_to_valid_file_state() -> Result<()> {
    let fixture = Fixture::new()?;
    fixture.configure(json!({ "title": "valid-file-fallback-must-not-render" }))?;
    let mut backend = Backend::start(vec![
        (
            200,
            r#"{"state":{"title":"backend-one","tokens":{"custom":"api-custom"}}}"#,
        ),
        (200, r#"{"title":"backend-two","visible":true}"#),
        (503, r#"{"title":"must-not-render"}"#),
        (200, r#"{"state":null}"#),
    ])?;
    let mut command = fixture.command(0);
    command
        .arg("--api-port")
        .arg(backend.port.to_string())
        .args(["--api-state-errors", "strict", "--header", "X-Test: strict"]);
    let mut server = Server::spawn(command)?;
    let address = server.listening()?;
    let mut events = Events::connect(address)?;
    events.expect("reload")?;

    let first = request(address, "/path%2Fone?q=a%20b", "text/html")?;
    backend.expect_request("/path%2Fone?q=a%20b")?;
    assert_eq!(first.status, 200, "{}", first.body);
    assert!(first.body.contains("backend-one"));
    assert!(first.body.contains("--brand: #112233;"));
    assert!(first.body.contains("api-custom"));

    let second = request(address, "/index.html?q=%25", "text/html")?;
    backend.expect_request("/?q=%25")?;
    assert_eq!(second.status, 200);
    assert!(second.body.contains("backend-two"));
    assert!(!second.body.contains("backend-one"));
    for (target, accept) in [
        ("/failed", "text/html"),
        ("/invalid-envelope", "application/json"),
    ] {
        let response = request(address, target, accept)?;
        backend.expect_request(target)?;
        assert_eq!(response.status, 502, "{}", response.body);
        assert!(response.header("cache-control").contains("no-store"));
        assert_eq!(response.header("x-test"), "strict");
        for forbidden in [
            "valid-file-fallback-must-not-render",
            "must-not-render",
            "backend-one",
            "backend-two",
        ] {
            assert!(!response.body.contains(forbidden), "{}", response.body);
        }
    }
    backend.finish()?;
    server.shutdown()
}

#[test]
fn native_api_forwarding_stays_live_while_pending_and_failed() -> Result<()> {
    let fixture = Fixture::new()?;
    fixture.configure(json!({ "fail": true }))?;
    let mut backend = Backend::start(vec![
        (200, r#"{"ok":"pending"}"#),
        (200, r#"{"ok":"failed"}"#),
    ])?;
    let mut command = fixture.command(0);
    command
        .arg("--api-port")
        .arg(backend.port.to_string())
        .args(["--header", "X-Test: proxy"]);
    let mut server = Server::spawn(command)?;
    let address = server.listening()?;
    let mut events = Events::connect(address)?;
    for state in ["pending", "failed"] {
        let response = request(address, "/api/check?q=a%20b", "application/json")?;
        backend.expect_request("/api/check?q=a%20b")?;
        assert_eq!(response.status, 200, "{}", response.body);
        assert!(response.body.contains(state));
        assert_eq!(response.header("x-test"), "proxy");
        assert_eq!(response.header("content-security-policy"), "");
        if state == "pending" {
            events.expect("reload-error")?;
            assert_gate(address, 500)?;
        }
    }
    backend.finish()?;
    server.shutdown()
}
