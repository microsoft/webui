// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Actual WKWebView/WebView2 owned-listener source fixture. Build browser JS
//! with `node tests/fixtures/native-ipc/build-local.mjs /tmp/local-renderer.js`,
//! then run this example with that output path as its sole argument.

mod generated {
    #![allow(dead_code)]
    include!("generated/rust/ipc.rs");
}

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use webui_desktop::{
    bind_owned_local_server,
    ipc::{IpcError, IpcErrorCode, IpcOptions},
    local_ipc_runtime_asset, DesktopApp, DesktopEvent, EventResponse, HostLifetime, IpcRegistry,
    LocalServerOptions, LoopbackOrigin, WindowHandle, LOCAL_IPC_RUNTIME_PATH,
};

const PAGE: &[u8] = b"<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>Owned native IPC</title><body><script type=\"module\" src=\"/assets/local-renderer.js\"></script></body></html>";

fn failure(message: &'static str) -> IpcError {
    IpcError::new(
        IpcErrorCode::Handler,
        message,
        "inspect the local native fixture",
    )
}

fn serve(
    listener: TcpListener,
    javascript: Vec<u8>,
    window: Arc<OnceLock<WindowHandle>>,
    stopped: Arc<AtomicBool>,
    passed: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
) {
    while !stopped.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut input = [0; 4096];
                let Ok(size) = stream.read(&mut input) else {
                    continue;
                };
                let header = &input[..size];
                let route = if header.starts_with(b"GET / ") || header.starts_with(b"GET /next ") {
                    (200, "text/html", PAGE)
                } else if header.starts_with(b"GET /assets/local-renderer.js ") {
                    (200, "text/javascript", javascript.as_slice())
                } else if header.starts_with(format!("GET {LOCAL_IPC_RUNTIME_PATH} ").as_bytes()) {
                    (
                        200,
                        "text/javascript; charset=utf-8",
                        local_ipc_runtime_asset(),
                    )
                } else if header.starts_with(b"POST /pass ") {
                    if calls.load(Ordering::Acquire) == 2 {
                        passed.store(true, Ordering::Release);
                    }
                    (200, "text/plain", b"reported".as_slice())
                } else if header.starts_with(b"POST /fail ") {
                    eprintln!(
                        "LOCAL_IPC_BROWSER_FAILURE {}",
                        String::from_utf8_lossy(header)
                    );
                    (500, "text/plain", b"failed".as_slice())
                } else {
                    eprintln!(
                        "LOCAL_IPC_UNEXPECTED_HTTP {}",
                        String::from_utf8_lossy(header)
                    );
                    (404, "text/plain", b"denied".as_slice())
                };
                let (status, mime, body) = route;
                let response = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: {mime}\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(body);
                if header.starts_with(b"POST /pass ") || header.starts_with(b"POST /fail ") {
                    if let Some(window) = window.get() {
                        let _ = window.request_close();
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                eprintln!("LOCAL_IPC_HTTP_FAILURE {error}");
                break;
            }
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("pass the built local-renderer.js path")?;
    let javascript = std::fs::read(path)?;
    let listener = bind_owned_local_server("127.0.0.1:0".parse()?)?;
    let origin = LoopbackOrigin::from_socket_addr(listener.local_addr()?)?;
    let server_socket = listener.try_clone()?;
    server_socket.set_nonblocking(true)?;
    let window = Arc::new(OnceLock::new());
    let stopped = Arc::new(AtomicBool::new(false));
    let passed = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let generation = Arc::new(AtomicU64::new(0));
    let (owner, lifetime) = HostLifetime::new();
    let server = {
        let window = Arc::clone(&window);
        let stopped = Arc::clone(&stopped);
        let passed = Arc::clone(&passed);
        let calls = Arc::clone(&calls);
        std::thread::spawn(move || serve(server_socket, javascript, window, stopped, passed, calls))
    };
    let mut registry = IpcRegistry::new(&generated::SCHEMA);
    let counts = Arc::clone(&calls);
    let observed_generation = Arc::clone(&generation);
    registry.register::<generated::host::Save, _, _>(move |context, request| {
        let counts = Arc::clone(&counts);
        let generation = Arc::clone(&observed_generation);
        async move {
            if request.id != u64::MAX
                || request.image.len() != 262144
                || request
                    .image
                    .iter()
                    .enumerate()
                    .any(|(i, byte)| usize::from(*byte) != (i * 31 + 7) % 256)
            {
                return Err(failure("generated protobuf bytes changed"));
            }
            let observed = context.session.generation();
            let previous = generation.load(Ordering::Acquire);
            if request.phase == "/" && previous != 0 {
                return Err(failure("first main document was admitted twice"));
            }
            if request.phase == "/next" && (previous == 0 || observed <= previous) {
                return Err(failure("replacement document reused the prior generation"));
            }
            if request.phase != "/" && request.phase != "/next" {
                return Err(failure("unexpected request phase"));
            }
            generation.store(observed, Ordering::Release);
            counts.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }
    })?;
    let frame = DesktopApp::from_local_server(LocalServerOptions::new(origin, lifetime))
        .application_ipc(
            &listener,
            registry,
            IpcOptions::for_schema(&generated::SCHEMA),
        )?
        .build()?;
    window
        .set(frame.window_handle().clone())
        .map_err(|_| "duplicate native window")?;
    let reported = Arc::clone(&passed);
    let counted = Arc::clone(&calls);
    frame.on_event(move |event| {
        if matches!(event, DesktopEvent::WindowClosed { .. }) {
            if !reported.load(Ordering::Acquire) || counted.load(Ordering::Acquire) != 2 {
                eprintln!("LOCAL_IPC_FAILURE window closed without two authenticated document RPCs");
                std::process::exit(2);
            }
            println!("LOCAL_IPC_RESULT source=owned native_carrier=1 documents=2 subframe_denied=true http_ipc_frames=0");
        }
        EventResponse::Continue
    })?;
    let result = webui_desktop::run_local_server_frame(frame);
    stopped.store(true, Ordering::Release);
    if let Err(error) = owner.revoke() {
        eprintln!("LOCAL_IPC_REVOKE {error}");
    }
    server.join().map_err(|_| "HTTP fixture thread panicked")?;
    result?;
    if !passed.load(Ordering::Acquire) || calls.load(Ordering::Acquire) != 2 {
        return Err("local native IPC did not complete both document calls".into());
    }
    println!("LOCAL_IPC_RESULT source=owned native_carrier=1 documents=2 subframe_denied=true http_ipc_frames=0");
    Ok(())
}
