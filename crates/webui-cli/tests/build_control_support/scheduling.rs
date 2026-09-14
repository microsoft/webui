// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::thread;
use std::time::Duration;

use anyhow::Result;
use serde_json::json;

use super::build_control_support::{assert_gate, request, Events, Fixture, Server};

#[test]
fn native_changed_sources_supersede_success_and_failure_with_one_coalesced_followup() -> Result<()>
{
    for stale_fails in [false, true] {
        let fixture = Fixture::new()?;
        let mut server = Server::spawn(fixture.command(0))?;
        let address = server.listening()?;
        let mut events = Events::connect(address)?;
        events.expect("reload")?;
        fixture.configure(json!({
            "title": "superseded", "delayMs": 2200, "fail": stale_fails
        }))?;
        fixture.wait_event("begin", "superseded")?;
        assert_gate(address, 503)?;

        // Separate native debounce batches, all while the same hook is awaiting.
        for index in 0..6 {
            fixture.configure(json!({
                "title": format!("burst-{index}"), "delayMs": 900
            }))?;
            thread::sleep(Duration::from_millis(70));
        }
        fixture.write_source("changed-source")?;
        fixture.configure(json!({
            "title": "current-build", "brand": "#778899", "delayMs": 900
        }))?;
        events.quiet()?;
        fixture.wait_event("end", "superseded")?;
        fixture.wait_event("begin", "current-build")?;
        assert_gate(address, 503)?;
        events.quiet()?;
        events.expect("reload")?;
        let response = request(address, "/", "text/html")?;
        assert_eq!(response.status, 200);
        assert!(response.body.contains("changed-source"));
        assert!(response.body.contains("current-build"));
        assert!(!response.body.contains("superseded"));
        assert!(!response.body.contains("burst-"));
        assert!(request(address, "/client.js", "*/*")?
            .body
            .contains("current-build"));
        server.assert_alive(Duration::from_millis(800))?;
        events.quiet()?;
        assert_eq!(
            fixture.count("begin")?,
            3,
            "one coalesced follow-up, not six builds"
        );
        fixture.assert_serial()?;
        server.shutdown()?;
    }
    Ok(())
}

#[test]
fn native_source_pending_extra_file_and_directory_inputs_trigger_builds() -> Result<()> {
    let fixture = Fixture::new()?;
    fs::write(fixture.source.join("client.ts"), b"export const count = 1;")?;
    let cli_file = fixture.extra_file.with_file_name("cli-input.txt");
    let cli_directory = fixture.extra_dir.with_file_name("cli-shared");
    fs::write(&cli_file, b"initial input")?;
    fs::create_dir(&cli_directory)?;
    fs::write(cli_directory.join("shared.ts"), b"export const value = 1;")?;
    let mut command = fixture.command(0);
    command.args([
        "--watch-path",
        "../cli-input.txt",
        "--watch-path",
        "../cli-shared",
    ]);
    let mut server = Server::spawn(command)?;
    let address = server.listening()?;
    let mut events = Events::connect(address)?;
    events.expect("reload")?;
    fixture.write_source("source-revision")?;
    events.expect("reload")?;
    assert!(request(address, "/", "text/html")?
        .body
        .contains("source-revision"));

    for path in [
        fixture.source.join("client.ts"),
        fixture.extra_file.clone(),
        fixture.extra_dir.join("shared.ts"),
        cli_file,
        cli_directory.join("shared.ts"),
    ] {
        let count = fixture.count("begin")?;
        fs::write(path, b"// changed dependency content")?;
        events.expect("reload")?;
        events.quiet()?;
        assert_eq!(fixture.count("begin")?, count + 1);
    }
    let count = fixture.count("begin")?;
    fs::write(
        fixture.extra_file.with_file_name("unrelated-sibling.json"),
        b"{}",
    )?;
    server.assert_alive(Duration::from_millis(800))?;
    events.quiet()?;
    assert_eq!(
        fixture.count("begin")?,
        count,
        "explicit file watch must not watch siblings"
    );
    fixture.assert_serial()?;
    server.shutdown()
}

#[test]
fn native_template_failure_never_publishes_new_client_and_recovers() -> Result<()> {
    let fixture = Fixture::new()?;
    let mut server = Server::spawn(fixture.command(0))?;
    let address = server.listening()?;
    let mut events = Events::connect(address)?;
    events.expect("reload")?;
    fs::write(
        fixture.source.join("test-card").join("test-card.html"),
        "<if><p>Invalid conditional</p></if>",
    )?;
    events.expect("reload-error")?;
    assert_gate(address, 500)?;
    events.quiet()?;
    fs::write(
        fixture.source.join("test-card").join("test-card.html"),
        "<article><h2>{{title}}</h2><p>repaired-template</p></article>",
    )?;
    events.expect("reload")?;
    let response = request(address, "/", "text/html")?;
    assert_eq!(response.status, 200);
    assert!(response.body.contains("repaired-template"));
    events.quiet()?;
    server.shutdown()
}
