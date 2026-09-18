// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::net::TcpListener;
use std::process::Command;
use std::thread;
use std::time::Duration;
use webui_test_utils::dev_server::TestServer;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn supervised_cli_preserves_error_status_and_single_json_diagnostic() -> TestResult {
    let root = tempfile::tempdir()?;
    for timeout in [None, Some("1")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_webui"));
        command
            .current_dir(root.path())
            .args(["serve", "missing", "--format", "json"]);
        if let Some(timeout) = timeout {
            command.args(["--shutdown-timeout", timeout]);
        }
        let output = command.output()?;
        assert_eq!(output.status.code(), Some(66));
        let text = String::from_utf8(output.stdout)?;
        assert_eq!(text.lines().count(), 1, "{text}");
        let diagnostic: serde_json::Value = serde_json::from_str(&text)?;
        assert_eq!(diagnostic["severity"], "error");
        assert!(!text.contains('\x1b'));
    }
    Ok(())
}

#[test]
fn unrepresentable_timeout_is_structured_and_does_not_start_building() -> TestResult {
    let root = tempfile::tempdir()?;
    let output = Command::new(env!("CARGO_BIN_EXE_webui"))
        .current_dir(root.path())
        .args([
            "serve",
            "--shutdown-timeout",
            "18446744073709551615",
            "--format",
            "json",
        ])
        .output()?;
    assert!(!output.status.success());
    let diagnostic: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(diagnostic["code"], "shutdown-invalid-policy");
    assert!(diagnostic["help"].is_string());
    assert_eq!(fs::read_dir(root.path())?.count(), 0);
    Ok(())
}

#[test]
fn actual_server_gates_startup_and_joins_after_live_rebuild() -> TestResult {
    for watch in [false, true] {
        let root = tempfile::tempdir()?;
        let entry = root.path().join("index.html");
        fs::write(&entry, "<html><body>before-rebuild</body></html>")?;
        let reservation = TcpListener::bind(("127.0.0.1", 0))?;
        let address = reservation.local_addr()?;
        let mut command = Command::new(env!("CARGO_BIN_EXE_webui"));
        command
            .current_dir(root.path())
            .env("WEBUI_NO_WATCH", "0")
            .args([
                "serve",
                "--shutdown-timeout",
                "30",
                "--port",
                &address.port().to_string(),
            ]);
        if watch {
            command.arg("--watch");
        }
        let mut server = TestServer::spawn_gated(&mut command)?;
        thread::sleep(Duration::from_millis(100));
        server.ensure_running()?;
        assert!(
            server.diagnostics()?.is_empty(),
            "startup gate emitted application output"
        );
        drop(reservation);
        server.send(b'G')?;
        server.wait_for_content(address, "before-rebuild")?;
        fs::write(&entry, "<html><body>after-rebuild</body></html>")?;
        if watch {
            server.wait_for_content(address, "after-rebuild")?;
        } else {
            server.wait_for_content(address, "before-rebuild")?;
        }
        server.send(b'S')?;
        assert_eq!(
            server.wait()?.code(),
            Some(125),
            "{}",
            server.diagnostics()?
        );
        assert!(std::net::TcpStream::connect(address).is_err());
    }
    Ok(())
}

#[test]
fn lost_controller_is_not_reported_as_success() -> TestResult {
    let root = tempfile::tempdir()?;
    fs::write(
        root.path().join("index.html"),
        "<html><body>ready</body></html>",
    )?;
    let reservation = TcpListener::bind(("127.0.0.1", 0))?;
    let address = reservation.local_addr()?;
    let mut command = Command::new(env!("CARGO_BIN_EXE_webui"));
    command.current_dir(root.path()).args([
        "serve",
        "--watch",
        "--shutdown-timeout",
        "30",
        "--port",
        &address.port().to_string(),
    ]);
    let mut server = TestServer::spawn_gated(&mut command)?;
    drop(reservation);
    server.send(b'G')?;
    server.wait_for_content(address, "ready")?;
    server.close_control();
    let status = server.wait()?;
    assert!(!status.success());
    assert_ne!(status.code(), Some(125));
    Ok(())
}

#[cfg(unix)]
#[test]
fn default_and_supervised_os_shutdown_keep_normal_exit_successful() -> TestResult {
    for supervised in [false, true] {
        let signals: &[&str] = if supervised {
            &["INT", "TERM", "QUIT", "HUP"]
        } else {
            &["INT", "TERM", "QUIT"]
        };
        for signal in signals {
            let root = tempfile::tempdir()?;
            fs::write(
                root.path().join("index.html"),
                "<html><body>ready</body></html>",
            )?;
            let reservation = TcpListener::bind(("127.0.0.1", 0))?;
            let address = reservation.local_addr()?;
            let mut command = Command::new(env!("CARGO_BIN_EXE_webui"));
            command.current_dir(root.path()).args([
                "serve",
                "--watch",
                "--port",
                &address.port().to_string(),
            ]);
            if supervised {
                command.args(["--shutdown-timeout", "5"]);
            }
            let mut server = TestServer::spawn(&mut command)?;
            drop(reservation);
            server.wait_for_content(address, "ready")?;
            server.signal(signal)?;
            assert!(server.wait()?.success(), "{}", server.diagnostics()?);
        }
    }
    Ok(())
}
