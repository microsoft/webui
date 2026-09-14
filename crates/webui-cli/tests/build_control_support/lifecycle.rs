// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::net::TcpListener;
use std::time::Duration;

use anyhow::Result;
use serde_json::json;

use super::build_control_support::{assert_gate, request, wait_status, Events, Fixture, Server};

#[test]
fn native_factory_and_initial_rebuild_are_awaited_after_real_listening_url() -> Result<()> {
    let fixture = Fixture::new()?;
    fixture.configure(json!({ "initDelayMs": 900 }))?;
    let mut server = Server::spawn(fixture.command(0))?;
    let address = server.listening()?;
    assert_eq!(
        fixture.count("end")?,
        0,
        "URL must precede initial build completion"
    );
    let mut events = Events::connect(address)?;
    assert_gate(address, 503)?;
    events.quiet()?;
    events.expect("reload")?;
    assert_eq!(request(address, "/", "text/html")?.status, 200);
    assert_eq!(fixture.count("initialized")?, 1);
    assert_eq!(fixture.count("end")?, 1);
    fixture.assert_serial()?;
    events.quiet()?;
    server.shutdown()?;
    assert_eq!(fixture.count("dispose-end")?, 1);
    Ok(())
}

#[test]
fn native_eof_is_orderly_and_disposal_is_awaited_before_exit() -> Result<()> {
    let fixture = Fixture::new()?;
    fixture.configure(json!({ "disposeDelayMs": 700 }))?;
    let mut server = Server::spawn(fixture.command(0))?;
    let address = server.listening()?;
    let mut events = Events::connect(address)?;
    events.expect("reload")?;
    server.close_stdin();
    fixture.wait_event("dispose-begin", "initial")?;
    server.assert_alive(Duration::from_millis(250))?;
    assert!(
        TcpListener::bind(address).is_ok(),
        "listener must stop before awaited disposal"
    );
    server.wait_exit(true)?;
    assert_eq!(fixture.count("dispose-begin")?, 1);
    assert_eq!(fixture.count("dispose-end")?, 1);
    Ok(())
}

#[test]
fn native_eof_during_initialization_or_rebuild_reaps_and_reuses_port() -> Result<()> {
    for initializing in [true, false] {
        let fixture = Fixture::new()?;
        fixture.configure(json!({
            "initDelayMs": if initializing { 700 } else { 0 }, "delayMs": 700
        }))?;
        let mut server = Server::spawn(fixture.command(0))?;
        let address = server.listening()?;
        fixture.wait_event(if initializing { "factory" } else { "begin" }, "initial")?;
        server.shutdown()?;
        let initialized = fixture.count("initialized")?;
        // Cancellation before the factory returns has no hooks to dispose.
        assert_eq!(
            fixture.count("dispose-end")?,
            initialized,
            "initializing={initializing}; calls: {:?}",
            fixture.events()?
        );
        assert_eq!(
            fixture.count("dispose-begin")?,
            initialized,
            "initializing={initializing}; calls: {:?}",
            fixture.events()?
        );
        if !initializing {
            assert_eq!(initialized, 1);
        }
        let mut restarted = Server::spawn(fixture.command(address.port()))?;
        assert_eq!(restarted.listening()?, address);
        restarted.shutdown()?;
    }
    Ok(())
}

#[test]
fn native_initialization_and_rebuild_deadlines_are_fatal() -> Result<()> {
    for initializing in [true, false] {
        let fixture = Fixture::new()?;
        fixture.configure(json!({
            "initDelayMs": if initializing { 5000 } else { 0 }, "delayMs": 5000
        }))?;
        let mut command = fixture.command(0);
        command.args(["--client-build-timeout-ms", "600"]);
        let mut server = Server::spawn(command)?;
        server.listening()?;
        server.failed("timeout")?;
    }
    Ok(())
}

#[test]
fn native_followup_deadline_is_fatal_not_a_recoverable_build_failure() -> Result<()> {
    let fixture = Fixture::new()?;
    fixture.configure(json!({ "delayMs": 300 }))?;
    let mut command = fixture.command(0);
    command.args(["--client-build-timeout-ms", "2000"]);
    let mut server = Server::spawn(command)?;
    let address = server.listening()?;
    let mut events = Events::connect(address)?;
    events.expect("reload")?;
    fixture.configure(json!({ "title": "hang", "delayMs": 6000 }))?;
    fixture.wait_event("begin", "hang")?;
    assert_gate(address, 503)?;
    server.failed("timeout")
}

#[test]
fn native_published_and_recoverable_failure_have_no_idle_timeout() -> Result<()> {
    let fixture = Fixture::new()?;
    let mut command = fixture.command(0);
    command.args(["--client-build-timeout-ms", "2200"]);
    let mut server = Server::spawn(command)?;
    let address = server.listening()?;
    let mut events = Events::connect(address)?;
    events.expect("reload")?;
    server.assert_alive(Duration::from_millis(2400))?;
    assert_eq!(request(address, "/", "text/html")?.status, 200);
    fixture.configure(json!({ "title": "recoverable", "fail": true, "delayMs": 100 }))?;
    events.expect("reload-error")?;
    server.assert_alive(Duration::from_millis(2400))?;
    assert_gate(address, 500)?;
    events.quiet()?;
    server.shutdown()
}

#[test]
fn native_invalid_modules_and_factory_failures_are_fatal() -> Result<()> {
    for module in [
        "export const notDefault = 1;",
        "export default {};",
        "export default async () => null;",
        "export default async () => ({ async dispose() {} });",
        "export default async () => ({ async rebuild() {} });",
        "export default async () => ({ rebuild: true, async dispose() {} });",
        "export default async () => ({ async rebuild() {}, dispose: false });",
        "export default async () => ({ async rebuild() {}, async dispose() {}, watchPaths: 'x' });",
        "export default async () => ({ async rebuild() {}, async dispose() {}, watchPaths: [3] });",
        "export default async () => { throw new Error('Factory creation failed'); };",
        "export default async (",
    ] {
        let fixture = Fixture::new()?;
        fs::write(&fixture.module, module)?;
        let mut server = Server::spawn(fixture.command(0))?;
        server.failed("builder")?;
        assert!(!fixture.dist.join("client.js").exists());
    }
    Ok(())
}

#[test]
fn native_missing_module_or_output_directory_fails_before_listening() -> Result<()> {
    for missing_module in [true, false] {
        let fixture = Fixture::new()?;
        if missing_module {
            fs::remove_file(&fixture.module)?;
        } else {
            fs::remove_dir(&fixture.dist)?;
        }
        let mut server = Server::spawn(fixture.command(0))?;
        server.wait_exit(false)?;
        server.no_listening_url();
        assert!(!server.logs().is_empty());
    }
    Ok(())
}

#[test]
fn native_disposal_failure_and_timeout_are_fatal_and_reaped() -> Result<()> {
    for hangs in [false, true] {
        let fixture = Fixture::new()?;
        fixture.configure(json!({
            "delayMs": 300, "disposeFailure": !hangs, "disposeHang": hangs
        }))?;
        let mut command = fixture.command(0);
        command.args(["--client-build-timeout-ms", "1500"]);
        let mut server = Server::spawn(command)?;
        let address = server.listening()?;
        let mut events = Events::connect(address)?;
        events.expect("reload")?;
        server.close_stdin();
        server.failed(if hangs { "timeout" } else { "dispos" })?;
        assert_eq!(fixture.count("dispose-begin")?, 1);
        assert_eq!(fixture.count("dispose-end")?, 0);
    }
    Ok(())
}

#[test]
fn native_worker_exit_during_factory_rebuild_or_idle_is_fatal() -> Result<()> {
    for phase in ["factory", "rebuild", "idle"] {
        let fixture = Fixture::new()?;
        fixture.configure(json!({
            "factoryCrash": phase == "factory", "crash": phase == "rebuild",
            "idleCrashMs": if phase == "idle" { 600 } else { 0 }
        }))?;
        let mut server = Server::spawn(fixture.command(0))?;
        let address = server.listening()?;
        if phase == "idle" {
            let mut events = Events::connect(address)?;
            events.expect("reload")?;
            assert_eq!(request(address, "/", "text/html")?.status, 200);
        }
        server.failed("exit")?;
    }
    Ok(())
}

#[test]
fn native_superseded_worker_crash_is_still_fatal() -> Result<()> {
    let fixture = Fixture::new()?;
    fixture.configure(json!({ "crash": true, "delayMs": 1600 }))?;
    let mut server = Server::spawn(fixture.command(0))?;
    let address = server.listening()?;
    fixture.wait_event("begin", "initial")?;
    fixture.configure(json!({ "title": "cannot-recover-dead-worker", "delayMs": 100 }))?;
    assert_gate(address, 503)?;
    server.failed("exit")?;
    assert_eq!(fixture.count("begin")?, 1);
    Ok(())
}

#[test]
fn native_port_collision_fails_without_advertising_or_stealing_the_port() -> Result<()> {
    let fixture = Fixture::new()?;
    let reserved = TcpListener::bind(("127.0.0.1", 0))?;
    let address = reserved.local_addr()?;
    let mut server = Server::spawn(fixture.command(address.port()))?;
    let status = server.wait_exit(false)?;
    server.no_listening_url();
    assert_eq!(status.code(), Some(69));
    assert!(server.logs().contains(&address.port().to_string()));
    assert!(TcpListener::bind(address).is_err());
    assert_eq!(fixture.count("factory")?, 0);
    drop(reserved);
    let mut restarted = Server::spawn(fixture.command(address.port()))?;
    assert_eq!(restarted.listening()?, address);
    restarted.shutdown()
}

#[test]
fn native_invalid_response_policy_and_options_fail_before_listening() -> Result<()> {
    let fixture = Fixture::new()?;
    for args in [
        vec!["--header", "Content-Length: 10"],
        vec!["--header", "X-Test: one", "--header", "x-test: two"],
        vec!["--header", "X-Test: good\r\nX-Injected: bad"],
        vec!["--csp", "script-src 'self'"],
        vec![
            "--csp",
            "script-src 'nonce-{nonce}'",
            "--header",
            "Content-Security-Policy: script-src 'self'",
        ],
        vec!["--client-build-timeout-ms", "0"],
        vec!["--build-control", "stdio"],
        vec!["--build-control-timeout-ms", "1000"],
    ] {
        let mut command = fixture.command(0);
        command.args(args);
        let mut server = Server::spawn(command)?;
        server.wait_exit(false)?;
        server.no_listening_url();
        assert!(!server.logs().is_empty());
    }
    Ok(())
}

#[test]
fn native_watch_off_builds_once_without_subscribing_or_injecting_reload() -> Result<()> {
    for environment_override in [false, true] {
        let fixture = Fixture::new()?;
        let mut command = fixture.command_with_watch(0, environment_override);
        if environment_override {
            command.env("WEBUI_NO_WATCH", "1");
        }
        let mut server = Server::spawn(command)?;
        let address = server.listening()?;
        assert_gate(address, 503)?;
        wait_status(address, 200)?;
        let response = request(address, "/", "text/html")?;
        assert!(!response.body.contains("new EventSource("));
        assert!(!response.body.contains("/__webui/livereload"));
        assert!(response.body.contains("initial"));
        let reload = request(address, "/__webui/livereload", "text/event-stream")?;
        assert!(!reload.header("content-type").contains("text/event-stream"));
        fixture.configure(json!({ "title": "must-not-rebuild", "delayMs": 100 }))?;
        fixture.write_source("must-not-recompile")?;
        fs::write(&fixture.extra_file, b"{}")?;
        server.assert_alive(Duration::from_millis(800))?;
        let unchanged = request(address, "/", "text/html")?;
        assert_eq!(unchanged.status, 200);
        assert!(unchanged.body.contains("initial"));
        assert!(!unchanged.body.contains("must-not-"));
        assert_eq!(fixture.count("begin")?, 1);
        fixture.assert_serial()?;
        server.shutdown()?;
        assert_eq!(fixture.count("dispose-end")?, 1);
    }
    Ok(())
}

#[test]
fn native_watch_off_initial_client_or_ssr_failure_exits_nonzero() -> Result<()> {
    for client_fails in [true, false] {
        let fixture = Fixture::new()?;
        fixture.configure(json!({
            "fail": client_fails, "mode": if client_fails { "" } else { "bad-state" }
        }))?;
        let mut server = Server::spawn(fixture.command_with_watch(0, false))?;
        server.listening()?;
        server.wait_exit(false)?;
        assert_eq!(fixture.count("dispose-end")?, 1);
    }
    Ok(())
}

#[test]
fn native_missing_node_is_actionable_without_changing_global_path() -> Result<()> {
    let fixture = Fixture::new()?;
    let mut command = fixture.command(0);
    command.env("PATH", &fixture.extra_dir);
    let mut server = Server::spawn(command)?;
    server.failed("node")?;
    assert!(server.logs().contains("PATH"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn native_ctrl_c_awaits_disposal_and_releases_port() -> Result<()> {
    let fixture = Fixture::new()?;
    let mut server = Server::spawn(fixture.command(0))?;
    let address = server.listening()?;
    let mut events = Events::connect(address)?;
    events.expect("reload")?;
    let signal = std::process::Command::new("kill")
        .args(["-INT", &server.pid().to_string()])
        .status()?;
    assert!(signal.success());
    server.wait_exit(true)?;
    assert_eq!(fixture.count("dispose-end")?, 1);
    Ok(())
}
