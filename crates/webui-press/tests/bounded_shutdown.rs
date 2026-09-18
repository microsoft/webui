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
fn supervisor_preserves_config_errors_without_duplicate_output() -> TestResult {
    let root = tempfile::tempdir()?;
    for timeout in [None, Some("1")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_webui-press"));
        command
            .current_dir(root.path())
            .args(["serve", "--config", "missing.json"]);
        if let Some(timeout) = timeout {
            command.args(["--shutdown-timeout", timeout]);
        }
        let output = command.output()?;
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8(output.stderr)?;
        assert_eq!(stderr.matches("Cannot read config").count(), 1, "{stderr}");
        assert_eq!(fs::read_dir(root.path())?.count(), 0);
    }
    Ok(())
}

#[test]
fn invalid_policy_does_not_extract_or_build() -> TestResult {
    let root = tempfile::tempdir()?;
    for timeout in ["0", "18446744073709551615"] {
        let output = Command::new(env!("CARGO_BIN_EXE_webui-press"))
            .current_dir(root.path())
            .args(["serve", "--shutdown-timeout", timeout])
            .output()?;
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("Cannot read config"));
        assert_eq!(fs::read_dir(root.path())?.count(), 0);
    }
    Ok(())
}

#[test]
fn actual_press_gates_builds_preserves_show_override_and_joins() -> TestResult {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("content"))?;
    fs::create_dir(root.path().join("template"))?;
    fs::create_dir(root.path().join("public"))?;
    fs::write(
        root.path().join("template/index.html"),
        "<html><body><aside>custom-shell</aside><main>{{{page.content}}}</main></body></html>",
    )?;
    let source = root.path().join("content/index.md");
    fs::write(&source, "# Before rebuild")?;
    let config = root.path().join("site.json");
    fs::write(
        &config,
        r#"{"site":{"title":"Shutdown"},"basePath":"/","contentDir":"content","outDir":"dist","publicDir":"public","show":"all","nav":[],"sidebar":[]}"#,
    )?;
    let reservation = TcpListener::bind(("127.0.0.1", 0))?;
    let address = reservation.local_addr()?;
    let mut command = Command::new(env!("CARGO_BIN_EXE_webui-press"));
    command
        .current_dir(root.path())
        .args([
            "serve",
            "--config",
            "site.json",
            "--show",
            "content",
            "--shutdown-timeout",
            "30",
            "--port",
            &address.port().to_string(),
        ])
        .arg("--template")
        .arg(root.path().join("template"));
    let mut server = TestServer::spawn_gated(&mut command)?;
    thread::sleep(Duration::from_millis(100));
    server.ensure_running()?;
    assert!(!root.path().join("dist").exists());
    assert!(
        server.diagnostics()?.is_empty(),
        "startup gate emitted application output"
    );
    drop(reservation);
    server.send(b'G')?;
    let response = server.wait_for_content(address, "Before rebuild")?;
    assert!(!response.contains("custom-shell"));
    fs::write(&source, "# After rebuild")?;
    let response = server.wait_for_content(address, "After rebuild")?;
    assert!(!response.contains("custom-shell"));
    server.send(b'S')?;
    assert_eq!(
        server.wait()?.code(),
        Some(125),
        "{}",
        server.diagnostics()?
    );
    let generated = fs::read_to_string(root.path().join("dist/index.html"))?;
    assert!(generated.contains("After rebuild"));
    assert!(!generated.contains("custom-shell"));
    assert!(std::net::TcpStream::connect(address).is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn default_and_supervised_signals_stop_the_real_press_server() -> TestResult {
    for supervised in [false, true] {
        let signals: &[&str] = if supervised {
            &["INT", "TERM", "QUIT", "HUP"]
        } else {
            &["INT", "TERM", "QUIT"]
        };
        for signal in signals {
            let root = tempfile::tempdir()?;
            fs::create_dir(root.path().join("content"))?;
            fs::create_dir(root.path().join("template"))?;
            fs::create_dir(root.path().join("public"))?;
            fs::write(
                root.path().join("template/index.html"),
                "<html><body>{{{page.content}}}</body></html>",
            )?;
            fs::write(root.path().join("content/index.md"), "# Ready")?;
            fs::write(
                root.path().join("site.json"),
                r#"{"site":{"title":"Shutdown"},"basePath":"/","contentDir":"content","outDir":"dist","publicDir":"public","nav":[],"sidebar":[]}"#,
            )?;
            let reservation = TcpListener::bind(("127.0.0.1", 0))?;
            let address = reservation.local_addr()?;
            let mut command = Command::new(env!("CARGO_BIN_EXE_webui-press"));
            command
                .current_dir(root.path())
                .args([
                    "serve",
                    "--config",
                    "site.json",
                    "--port",
                    &address.port().to_string(),
                ])
                .arg("--template")
                .arg(root.path().join("template"));
            if supervised {
                command.args(["--shutdown-timeout", "5"]);
            }
            let mut server = TestServer::spawn(&mut command)?;
            drop(reservation);
            server.wait_for_content(address, "Ready")?;
            server.signal(signal)?;
            assert!(server.wait()?.success(), "{}", server.diagnostics()?);
        }
    }
    Ok(())
}
