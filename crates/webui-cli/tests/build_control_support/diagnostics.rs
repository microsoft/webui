// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::net::TcpListener;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use super::build_control_support::{Fixture, Server};

#[test]
fn native_json_diagnostics_report_no_watch_failures_and_preserve_lifecycle() -> Result<()> {
    for client_failure in [true, false] {
        let fixture = Fixture::new()?;
        fixture.configure(json!({
            "title": "json-failure", "fail": client_failure, "delayMs": 300
        }))?;
        if !client_failure {
            fs::write(
                fixture.source.join("test-card").join("test-card.html"),
                "<if><p>Invalid conditional</p></if>",
            )?;
        }
        // JSON mode is diagnostic-only; choose the port without a readiness protocol.
        let reserved = TcpListener::bind(("127.0.0.1", 0))?;
        let address = reserved.local_addr()?;
        drop(reserved);
        let mut command = fixture.command_with_watch(address.port(), false);
        command.args(["--format", "json"]);
        let mut server = Server::spawn(command)?;
        // Keep piped stdin open: the build failure, not EOF, must terminate this run.
        server.wait_exit(false)?;
        let stdout = server.stdout();
        assert!(
            !stdout.contains('\u{1b}'),
            "ANSI in JSON diagnostics: {stdout}"
        );
        let records: Vec<Value> = stdout
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()
            .context("stdout must contain only JSON diagnostics, not hook logs")?;
        assert_eq!(records.len(), 1, "one build error: {stdout}");
        let error = &records[0];
        assert_eq!(error["severity"], "error");
        assert!(error["message"]
            .as_str()
            .is_some_and(|message| !message.is_empty()));
        assert!(error["chain"]
            .as_array()
            .is_some_and(|chain| !chain.is_empty()));
        for key in ["code", "file", "line", "column", "snippet", "help"] {
            assert!(
                error.get(key).is_some(),
                "missing diagnostic field {key}: {error}"
            );
        }
        if client_failure {
            assert!(
                error["message"].as_str().is_some_and(
                    |message| message.contains("Client plugin failed after emitting files")
                ),
                "{error}"
            );
        } else {
            assert!(
                error["code"].as_str().is_some_and(|code| !code.is_empty()),
                "{error}"
            );
            assert!(
                error["help"].as_str().is_some_and(|help| !help.is_empty()),
                "{error}"
            );
        }
        assert!(server
            .logs()
            .contains("fixture factory stdout is not a protocol"));
        assert_eq!(fixture.count("factory")?, 1);
        assert_eq!(fixture.count("begin")?, 1);
        assert_eq!(fixture.count("dispose-begin")?, 1);
        assert_eq!(fixture.count("dispose-end")?, 1);
        let _reused = TcpListener::bind(address).context("JSON failure releases its listener")?;
    }
    Ok(())
}
