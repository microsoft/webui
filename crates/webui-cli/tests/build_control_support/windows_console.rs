// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#[path = "windows_console/child.rs"]
mod child;
#[path = "windows_console/platform.rs"]
mod platform;

use std::fs;
use std::io::{BufRead, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{ensure, Context, Result};

use super::build_control_support::{request, wait_status, Fixture, Server};
use child::ConsoleChild;
use platform::{broadcast, clear_inherited_ignore};

#[test]
fn native_windows_console_ctrl_c_ready_builtin() -> Result<()> {
    exercise_console("ready")
}

#[test]
fn native_windows_console_ctrl_c_recoverable_builtin_error() -> Result<()> {
    exercise_console("recoverable")
}

fn exercise_console(state: &str) -> Result<()> {
    // Neither the libtest runner nor the user's console ever broadcasts. The
    // controller and sentinel each get their own newly allocated real console.
    let mut sentinel = ConsoleChild::spawn(&format!("sentinel-{state}"))?;
    let address: SocketAddr = sentinel.receive("SENTINEL ")?.parse()?;
    ping(address)?;
    let mut controller = ConsoleChild::spawn(state)?;
    let cli_address: SocketAddr = controller.receive("ARMED ")?.parse()?;
    let processes = controller.processes()?;
    for name in ["webui.exe", "node.exe", "esbuild.exe"] {
        ensure!(
            processes.iter().any(|process| {
                PathBuf::from(&process.image)
                    .file_name()
                    .is_some_and(|image| image.eq_ignore_ascii_case(name))
            }),
            "real builtin process missing: {name}"
        );
    }
    for process in &processes {
        ensure!(
            !process.exited()?,
            "process {} exited before Ctrl+C",
            process.id
        );
        println!("owned before real Ctrl+C: {} {}", process.id, process.image);
    }
    controller.send("signal")?;
    let status = controller.wait()?;
    // Check cleanup and isolation even for the pinned, expected-to-fail binary.
    for process in &processes {
        ensure!(
            process.await_exit()?,
            "owned process {} survived Ctrl+C",
            process.id
        );
    }
    controller.assert_empty()?;
    let _released = TcpListener::bind(cli_address).context("reuse real Ctrl+C server port")?;
    ping(address)?;
    sentinel.send("stop")?;
    ensure!(sentinel.wait()?.success(), "unrelated sentinel failed");
    sentinel.assert_empty()?;
    println!(
        "{state}: owned processes exited, port released, separate-console sentinel responsive"
    );
    ensure!(
        status.success(),
        "real console regression failed ({status}):\n{}",
        controller.logs()?
    );
    Ok(())
}

// Re-executed only by ConsoleChild. A normal suite run does not spawn helpers.
#[test]
fn console_child() -> Result<()> {
    let Ok(role) = std::env::var("WEBUI_CONSOLE_TEST_ROLE") else {
        return Ok(());
    };
    let input = std::io::stdin();
    let mut input = input.lock();
    expect_command(&mut input, "start")?;
    if role.starts_with("sentinel-") {
        run_sentinel(&mut input)
    } else {
        run_controller(&role, &mut input)
    }
}

fn run_controller(state: &str, input: &mut impl BufRead) -> Result<()> {
    let fixture = Fixture::new()?;
    // Resolve the already installed workspace esbuild via Node's standard
    // NODE_PATH fallback. No install, mock esbuild, or private downstream app.
    let dependencies = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("packages")
        .join("webui")
        .join("node_modules");
    fs::write(fixture.source.join("package.json"), r#"{"type":"module"}"#)?;
    fs::write(
        fixture.source.join("index.html"),
        "<!doctype html><html><head><title>Console regression</title></head>\
         <body><h1>Generic builtin client</h1><script type=\"module\" src=\"/index.js\"></script></body></html>",
    )?;
    let entry = fixture.source.join("index.ts");
    fs::write(&entry, "console.log('generic builtin client');")?;
    let mut command = fixture.cli_command("dev");
    command
        .arg(&fixture.source)
        .args(["--port", "0", "--servedir"])
        .arg(&fixture.dist)
        .env("NODE_PATH", dependencies);
    clear_inherited_ignore()?;
    let mut server = Server::spawn(command)?;
    let address = server.listening()?;
    wait_status(address, 200)?;
    ensure!(
        request(address, "/index.js", "*/*")?.status == 200,
        "builtin output missing"
    );
    if state == "recoverable" {
        fs::write(entry, "export const broken = ;")?;
        wait_status(address, 500)?;
        ensure!(
            server.logs().contains("Client build failed"),
            "expected recoverable JS error"
        );
    } else {
        ensure!(state == "ready", "unexpected controller role");
    }
    server.assert_alive(Duration::from_millis(100))?;
    println!("ARMED {address}");
    std::io::stdout().flush()?;
    expect_command(input, "signal")?;
    let outer = std::env::var("WEBUI_CONSOLE_TEST_PARENT")?.parse()?;
    broadcast(server.pid(), outer)?;
    // Stdin deliberately stays OPEN: success must come from real Ctrl+C, not
    // the existing EOF shutdown path (which runs only for assertion cleanup).
    server.wait_exit(true)?;
    println!("REAL_CTRL_C_EXIT_0 {state}");
    Ok(())
}

fn run_sentinel(input: &mut impl BufRead) -> Result<()> {
    clear_inherited_ignore()?;
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(true)?;
    println!("SENTINEL {}", listener.local_addr()?);
    std::io::stdout().flush()?;
    // The child is contained by its own job and killed on any parent failure.
    let responder = std::thread::spawn(move || -> Result<()> {
        for _ in 0..2 {
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        ensure!(
                            std::time::Instant::now() < deadline,
                            "sentinel request timed out"
                        );
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => return Err(error.into()),
                }
            };
            stream.set_read_timeout(Some(Duration::from_secs(3)))?;
            stream.set_write_timeout(Some(Duration::from_secs(3)))?;
            let mut request = [0; 4];
            stream.read_exact(&mut request)?;
            ensure!(&request == b"ping", "unexpected sentinel request");
            stream.write_all(b"pong")?;
        }
        Ok(())
    });
    expect_command(input, "stop")?;
    responder
        .join()
        .map_err(|_| anyhow::anyhow!("sentinel responder panicked"))??;
    Ok(())
}

fn ping(address: SocketAddr) -> Result<()> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    stream.write_all(b"ping")?;
    let mut reply = [0; 4];
    stream.read_exact(&mut reply)?;
    ensure!(&reply == b"pong", "unrelated sentinel is not responsive");
    Ok(())
}

fn expect_command(input: &mut impl BufRead, expected: &str) -> Result<()> {
    let mut line = String::new();
    input.read_line(&mut line)?;
    ensure!(
        line.trim() == expected,
        "expected helper command {expected}, got {line:?}"
    );
    Ok(())
}
