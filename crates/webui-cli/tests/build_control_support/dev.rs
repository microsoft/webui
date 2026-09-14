// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::time::Duration;

use anyhow::Result;
use serde_json::json;

use super::build_control_support::{
    assert_gate, bootstrap, request, wait_status, Events, Fixture, Server,
};

#[test]
fn native_dev_help_exposes_builtin_and_custom_client_workflows() -> Result<()> {
    let fixture = Fixture::new()?;
    let mut command = fixture.dev_command(0, false);
    command.arg("--help");
    let mut server = Server::spawn(command)?;
    server.wait_exit(true)?;
    server.no_listening_url();
    let help = server.stdout();
    for flag in [
        "--client-entry",
        "--client-builder",
        "--watch-path",
        "--no-watch",
    ] {
        assert!(help.contains(flag), "missing {flag} in dev help: {help}");
    }
    assert_eq!(fixture.count("factory")?, 0);
    Ok(())
}

#[test]
fn native_dev_custom_hook_uses_default_webui_and_watch_unless_disabled() -> Result<()> {
    for no_watch in [false, true] {
        let fixture = Fixture::new()?;
        assert!(!fixture.source.join("index.ts").exists());
        let mut server = Server::spawn(fixture.dev_command(0, no_watch))?;
        let address = server.listening()?;
        assert_gate(address, 503)?;
        let mut events = if no_watch {
            wait_status(address, 200)?;
            None
        } else {
            let mut events = Events::connect(address)?;
            events.expect("reload")?;
            Some(events)
        };
        let initial = request(address, "/", "text/html")?;
        assert_eq!(bootstrap(&initial.body)?["state"]["title"], "initial");
        assert_eq!(initial.body.contains("new EventSource("), !no_watch);
        fixture.configure(json!({ "title": "dev-followup", "delayMs": 100 }))?;
        if let Some(events) = &mut events {
            events.expect("reload")?;
            assert!(request(address, "/", "text/html")?
                .body
                .contains("dev-followup"));
            events.quiet()?;
            assert_eq!(fixture.count("begin")?, 2);
        } else {
            server.assert_alive(Duration::from_millis(800))?;
            let unchanged = request(address, "/", "text/html")?;
            assert_eq!(unchanged.status, 200);
            assert!(unchanged.body.contains("initial"));
            assert!(!unchanged.body.contains("dev-followup"));
            assert_eq!(fixture.count("begin")?, 1);
        }
        fixture.assert_serial()?;
        server.shutdown()?;
        assert_eq!(fixture.count("dispose-end")?, 1);
    }
    Ok(())
}

#[test]
fn native_dev_unavailable_local_esbuild_has_actionable_install_guidance() -> Result<()> {
    let fixture = Fixture::new()?;
    fs::write(
        fixture.source.join("index.ts"),
        b"export const ready = true;",
    )?;
    let package = fixture.source.join("node_modules").join("esbuild");
    fs::create_dir_all(&package)?;
    // An incomplete local install shadows any workspace ancestor's real esbuild.
    fs::write(
        package.join("package.json"),
        br#"{"name":"esbuild","exports":"./not-installed.js"}"#,
    )?;
    let mut command = fixture.cli_command("dev");
    command
        .arg(&fixture.source)
        .args(["--port", "0", "--no-watch", "--servedir"])
        .arg(&fixture.dist);
    let mut server = Server::spawn(command)?;
    server.failed("esbuild")?;
    let logs = server.logs();
    assert!(logs.contains("devDependencies"), "{logs}");
    assert!(logs.contains("restart"), "{logs}");
    Ok(())
}

#[test]
fn native_dev_missing_client_entry_reports_the_input_and_remedy() -> Result<()> {
    let fixture = Fixture::new()?;
    let mut command = fixture.cli_command("dev");
    command
        .arg(&fixture.source)
        .args([
            "--port",
            "0",
            "--no-watch",
            "--client-entry",
            "absent.ts",
            "--servedir",
        ])
        .arg(&fixture.dist);
    let mut server = Server::spawn(command)?;
    server.failed("absent.ts")?;
    assert!(
        server.logs().contains("--client-entry") || server.logs().contains("Create"),
        "missing entry diagnostics must identify a remedy: {}",
        server.logs()
    );
    Ok(())
}
