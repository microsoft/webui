// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Real owned-listener GTK IPC journey. Kept in the existing Linux test's
//! single GUI thread so ordinary cargo test cannot race GTK main contexts.

mod generated {
    #![allow(dead_code)]
    #![allow(clippy::single_match)]
    include!("native-ipc/generated/rust/ipc.rs");
}

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use webui_desktop::{
    bind_owned_local_server,
    ipc::{IpcError, IpcErrorCode, IpcOptions},
    local_ipc_runtime_asset, DesktopApp, DesktopEvent, EventResponse, HostLifetime, IpcRegistry,
    LocalServerOptions, LoopbackOrigin, WindowHandle, LOCAL_IPC_RUNTIME_PATH,
};

const PAGE: &[u8] = b"<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>Linux owned local IPC</title><body><script type=\"module\" src=\"/assets/linux-renderer.js\"></script></body></html>";
const RENDERER: &[u8] = include_bytes!("linux-local-renderer.js");

struct Observed {
    calls: AtomicUsize,
    generation: AtomicU64,
    passed: AtomicBool,
    unexpected_http: AtomicUsize,
    closed: AtomicBool,
    stopped: AtomicBool,
    window: OnceLock<WindowHandle>,
}

fn failure(message: &'static str) -> IpcError {
    IpcError::new(
        IpcErrorCode::Handler,
        message,
        "inspect the Linux owned IPC fixture",
    )
}

fn serve(listener: TcpListener, observed: Arc<Observed>) {
    while !observed.stopped.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut bytes = [0_u8; 8192];
                let mut length = 0;
                while length < bytes.len()
                    && !bytes[..length].windows(4).any(|part| part == b"\r\n\r\n")
                {
                    match stream.read(&mut bytes[length..]) {
                        Ok(0) | Err(_) => break,
                        Ok(count) => length += count,
                    }
                }
                let Ok(request) = std::str::from_utf8(&bytes[..length]) else {
                    observed.unexpected_http.fetch_add(1, Ordering::AcqRel);
                    continue;
                };
                let line = request.split("\r\n").next().unwrap_or("");
                let (status, mime, body) = match line {
                    "GET / HTTP/1.1" | "GET /next HTTP/1.1" => (200, "text/html", PAGE),
                    "GET /assets/linux-renderer.js HTTP/1.1" => (200, "text/javascript", RENDERER),
                    "POST /pass HTTP/1.1" => {
                        if observed.calls.load(Ordering::Acquire) == 2
                            && observed.unexpected_http.load(Ordering::Acquire) == 0
                        {
                            observed.passed.store(true, Ordering::Release);
                        }
                        (204, "text/plain", b"".as_slice())
                    }
                    _ if line.starts_with("POST /fail?step=") => {
                        let diagnostic = line
                            .strip_prefix("POST /fail?step=")
                            .and_then(|value| value.strip_suffix(" HTTP/1.1"))
                            .filter(|value| {
                                value.len() <= 72
                                    && value.bytes().all(|byte| {
                                        byte.is_ascii_lowercase()
                                            || byte.is_ascii_digit()
                                            || byte == b'-'
                                            || byte == b'&'
                                            || byte == b'='
                                    })
                            })
                            .unwrap_or("invalid");
                        eprintln!("LINUX_IPC_BROWSER_FAILURE {diagnostic}");
                        (500, "text/plain", b"rejected".as_slice())
                    }
                    _ if line == format!("GET {LOCAL_IPC_RUNTIME_PATH} HTTP/1.1") => {
                        (200, "text/javascript", local_ipc_runtime_asset())
                    }
                    _ => {
                        // Browser favicon/probe requests may be ordinary HTTP.
                        // Only a reserved IPC endpoint would violate the
                        // private native lane; never print token-bearing URLs.
                        if line
                            .split_whitespace()
                            .nth(1)
                            .is_some_and(|path| path.starts_with("/_webui/ipc"))
                        {
                            observed.unexpected_http.fetch_add(1, Ordering::AcqRel);
                        }
                        eprintln!("LINUX_IPC_UNEXPECTED_HTTP_ROUTE");
                        (404, "text/plain", b"denied".as_slice())
                    }
                };
                let header = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: {mime}\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(body);
                if line == "POST /pass HTTP/1.1" || line.starts_with("POST /fail?step=") {
                    if let Some(window) = observed.window.get() {
                        let _ = window.request_close();
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                observed.unexpected_http.fetch_add(1, Ordering::AcqRel);
                eprintln!("LINUX_IPC_HTTP_FAILURE {error}");
                break;
            }
        }
    }
}

pub(super) fn run() -> Result<(), Box<dyn std::error::Error>> {
    let listener = bind_owned_local_server("127.0.0.1:0".parse()?)?;
    let address = listener.local_addr()?;
    let origin = LoopbackOrigin::from_socket_addr(address)?;
    let server_socket = listener.try_clone()?;
    server_socket.set_nonblocking(true)?;
    let observed = Arc::new(Observed {
        calls: AtomicUsize::new(0),
        generation: AtomicU64::new(0),
        passed: AtomicBool::new(false),
        unexpected_http: AtomicUsize::new(0),
        closed: AtomicBool::new(false),
        stopped: AtomicBool::new(false),
        window: OnceLock::new(),
    });
    let server = {
        let observed = Arc::clone(&observed);
        std::thread::spawn(move || serve(server_socket, observed))
    };
    let mut registry = IpcRegistry::new(&generated::SCHEMA);
    let calls = Arc::clone(&observed);
    registry.register::<generated::host::Save, _, _>(move |context, request| {
        let observed = Arc::clone(&calls);
        async move {
            if request.id != u64::MAX
                || request.image.len() != 262144
                || request
                    .image
                    .iter()
                    .enumerate()
                    .any(|(index, byte)| usize::from(*byte) != (index * 31 + 7) % 256)
            {
                return Err(failure("generated 256-KiB payload changed"));
            }
            let generation = context.session.generation();
            let previous = observed.generation.load(Ordering::Acquire);
            if (request.phase == "/" && previous != 0)
                || (request.phase == "/next" && (previous == 0 || generation <= previous))
                || !matches!(request.phase.as_str(), "/" | "/next")
            {
                return Err(failure("local IPC document reused a retired generation"));
            }
            observed.generation.store(generation, Ordering::Release);
            observed.calls.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }
    })?;
    let (owner, lifetime) = HostLifetime::new();
    let frame = DesktopApp::from_local_server(LocalServerOptions::new(origin, lifetime))
        .application_ipc(
            &listener,
            registry,
            IpcOptions::for_schema(&generated::SCHEMA),
        )?
        .build()?;
    let ipc = frame
        .ipc()
        .ok_or("owned Linux IPC handle was not installed")?;
    observed
        .window
        .set(frame.window_handle().clone())
        .map_err(|_| "duplicate Linux IPC native window")?;
    let watchdog = {
        let observed = Arc::clone(&observed);
        std::thread::spawn(move || {
            let until = Instant::now() + Duration::from_secs(35);
            while Instant::now() < until {
                if observed.closed.load(Ordering::Acquire) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            eprintln!("LINUX_IPC_FAILURE deadline without native window close");
            std::process::exit(2);
        })
    };
    let events = Arc::clone(&observed);
    let ipc_on_close = ipc.clone();
    frame.on_event(move |event| {
        if matches!(event, DesktopEvent::WindowClosed { .. }) {
            events.closed.store(true, Ordering::Release);
            if !events.passed.load(Ordering::Acquire)
                || events.calls.load(Ordering::Acquire) != 2
                || events.unexpected_http.load(Ordering::Acquire) != 0
                || ipc_on_close.current_session().is_ok()
            {
                eprintln!("LINUX_IPC_FAILURE native close before private RPCs or IPC retirement");
                std::process::exit(3);
            }
        }
        EventResponse::Continue
    })?;
    let result = webui_desktop::run_local_server_frame(frame);
    observed.stopped.store(true, Ordering::Release);
    server
        .join()
        .map_err(|_| "Linux IPC HTTP server panicked")?;
    watchdog.join().map_err(|_| "Linux IPC watchdog panicked")?;
    result?;
    assert!(
        observed.closed.load(Ordering::Acquire),
        "WindowClosed was not observed"
    );
    assert!(
        observed.passed.load(Ordering::Acquire),
        "Linux IPC fixture did not report PASS"
    );
    assert_eq!(observed.calls.load(Ordering::Acquire), 2);
    assert_eq!(observed.unexpected_http.load(Ordering::Acquire), 0);
    assert!(
        ipc.current_session().is_err(),
        "IPC session revived after native close"
    );
    owner.revoke()?;
    drop(listener);
    println!("LINUX_IPC_RPC_RESULT documents=2 generated_rpc_bytes=262144 http_ipc_frames=0");
    Ok(())
}

/// A separate permissive WebKitGTK view is required to execute an opaque
/// cross-origin child: the product frame deliberately denies child document
/// commits. It loads the *same generated mediator bytes* into a top-only
/// named isolated world and proves child controls/data with a known credential
/// never reach either native handler.
pub(super) fn prove_child_isolation() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = root.join("tests/fixtures/linux-local-isolation.c");
    let mediator =
        std::path::Path::new(env!("WEBUI_DESKTOP_IPC_ASSET_DIR")).join("linux-local-mediator.js");
    let temporary = tempfile::tempdir()?;
    let executable = temporary.path().join("linux-local-isolation");
    let flags = Command::new("pkg-config")
        .args(["--cflags", "--libs", "gtk4", "webkitgtk-6.0"])
        .output()?;
    if !flags.status.success() || flags.stdout.len() > 8192 {
        return Err("GTK/WebKitGTK development headers are unavailable".into());
    }
    let flags = String::from_utf8(flags.stdout)?;
    let mut compiler = Command::new("cc");
    compiler
        .args(["-Wall", "-Wextra", "-Werror", "-Wno-unused-parameter"])
        .arg(&source)
        .args(["-o"])
        .arg(&executable)
        .args(flags.split_whitespace());
    let compiled = compiler.output()?;
    if !compiled.status.success() {
        eprintln!(
            "LINUX_IPC_CHILD_BUILD_FAILURE {}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        return Err("Linux native child isolation fixture did not compile".into());
    }
    let result = Command::new(&executable).arg(&mediator).output()?;
    eprintln!("{}", String::from_utf8_lossy(&result.stdout).trim());
    if !result.status.success() {
        eprintln!(
            "LINUX_IPC_CHILD_FAILURE {}",
            String::from_utf8_lossy(&result.stderr)
        );
        return Err("cross-origin sandboxed child reached an isolated native handler".into());
    }
    Ok(())
}
