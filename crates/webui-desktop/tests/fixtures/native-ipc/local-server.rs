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
    local_ipc_runtime_asset, DesktopApp, DesktopEvent, EventResponse, HostLifetime,
    HttpFrameOrigin, IpcRegistry, LocalServerOptions, LoopbackOrigin, WindowHandle,
    LOCAL_IPC_RUNTIME_PATH,
};

const PAGE: &[u8] = b"<!doctype html><html lang=\"en\" data-preview=\"true\"><meta charset=\"utf-8\"><title>Owned native IPC</title><body><script type=\"module\" src=\"/assets/local-renderer.js\"></script></body></html>";
const LINUX_PAGE: &[u8] = b"<!doctype html><html lang=\"en\" data-preview=\"false\"><meta charset=\"utf-8\"><title>Owned native IPC</title><body><script type=\"module\" src=\"/assets/local-renderer.js\"></script></body></html>";
const PREVIEW_PAGE: &[u8] = b"<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>Unprivileged preview</title><body><script type=\"module\" src=\"/assets/local-renderer.js\"></script></body></html>";
const PREVIEW_RECEIPTS_REQUIRED: usize = if cfg!(target_os = "linux") { 0 } else { 1 };

fn header<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request
        .split("\r\n")
        .skip(1)
        .take_while(|line| !line.is_empty())
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name).then_some(value.trim())
        })
}

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
    preview_host: String,
    preview_origin: String,
    main_host: String,
) {
    let wrong_host = preview_host.replacen("p-alpha.", "p-beta.", 1);
    let preview_gets = Arc::new(AtomicUsize::new(0));
    let deep_gets = Arc::new(AtomicUsize::new(0));
    let asset_gets = Arc::new(AtomicUsize::new(0));
    let receipts = Arc::new(AtomicUsize::new(0));
    let wrong_lease_gets = Arc::new(AtomicUsize::new(0));
    while !stopped.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut input = [0; 4096];
                let mut size = 0;
                while size < input.len()
                    && !input[..size].windows(4).any(|bytes| bytes == b"\r\n\r\n")
                {
                    match stream.read(&mut input[size..]) {
                        Ok(0) | Err(_) => break,
                        Ok(count) => size += count,
                    }
                }
                if size == 0 {
                    continue;
                }
                if !input[..size].windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    eprintln!("LOCAL_IPC_HTTP_FAILURE incomplete or oversized request headers");
                    continue;
                }
                let Ok(request) = std::str::from_utf8(&input[..size]) else {
                    continue;
                };
                let line = request.split("\r\n").next().unwrap_or("");
                let host = header(request, "Host").unwrap_or("");
                let request_origin = header(request, "Origin");
                let preview = host == preview_host;
                let main = host == main_host;
                if host == wrong_host && line.starts_with("GET /preview") {
                    wrong_lease_gets.fetch_add(1, Ordering::AcqRel);
                }
                // Never dump raw request headers: the native credential must not
                // accidentally appear in an HTTP diagnostic.
                let route = if preview
                    && request_origin.is_some_and(|value| value != preview_origin)
                {
                    (403, "text/plain", b"wrong origin".as_slice())
                } else if preview && line == "GET /preview?lease=alpha HTTP/1.1" {
                    preview_gets.fetch_add(1, Ordering::AcqRel);
                    (200, "text/html", PREVIEW_PAGE)
                } else if preview && line == "GET /preview/deep?lease=alpha HTTP/1.1" {
                    deep_gets.fetch_add(1, Ordering::AcqRel);
                    (200, "text/html", PREVIEW_PAGE)
                } else if preview
                    && line == "POST /preview/receipt HTTP/1.1"
                    && request_origin == Some(preview_origin.as_str())
                    && header(request, "Content-Length").is_none_or(|length| length == "0")
                {
                    receipts.fetch_add(1, Ordering::AcqRel);
                    (200, "text/plain", b"recorded".as_slice())
                } else if main && (line == "GET / HTTP/1.1" || line == "GET /next HTTP/1.1") {
                    (
                        200,
                        "text/html",
                        if cfg!(target_os = "linux") {
                            LINUX_PAGE
                        } else {
                            PAGE
                        },
                    )
                } else if (main || preview) && line == "GET /assets/local-renderer.js HTTP/1.1" {
                    if preview {
                        asset_gets.fetch_add(1, Ordering::AcqRel);
                    }
                    (200, "text/javascript", javascript.as_slice())
                } else if (main || preview)
                    && line == format!("GET {LOCAL_IPC_RUNTIME_PATH} HTTP/1.1")
                {
                    (
                        200,
                        "text/javascript; charset=utf-8",
                        local_ipc_runtime_asset(),
                    )
                } else if main && line == "POST /pass HTTP/1.1" {
                    if calls.load(Ordering::Acquire) == 2
                        && (cfg!(target_os = "linux")
                            || (preview_gets.load(Ordering::Acquire) == 1
                                && deep_gets.load(Ordering::Acquire) == 1
                                && asset_gets.load(Ordering::Acquire) >= 2
                                && receipts.load(Ordering::Acquire) == 1
                                && wrong_lease_gets.load(Ordering::Acquire) == 0))
                    {
                        passed.store(true, Ordering::Release);
                    } else {
                        eprintln!("LOCAL_IPC_PREVIEW_FAILURE missing exact preview document/asset/receipt");
                    }
                    (200, "text/plain", b"reported".as_slice())
                } else if main && line == "POST /fail HTTP/1.1" {
                    eprintln!("LOCAL_IPC_BROWSER_FAILURE (renderer reported failure)");
                    (500, "text/plain", b"failed".as_slice())
                } else {
                    eprintln!("LOCAL_IPC_UNEXPECTED_HTTP (unrecognized route or host)");
                    (404, "text/plain", b"denied".as_slice())
                };
                let (status, mime, body) = route;
                let response = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: {mime}\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(body);
                if main && (line == "POST /pass HTTP/1.1" || line == "POST /fail HTTP/1.1") {
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
    eprintln!(
        "LOCAL_IPC_HTTP_COUNTS preview={} deep={} assets={} receipts={} wrong_lease={}",
        preview_gets.load(Ordering::Acquire),
        deep_gets.load(Ordering::Acquire),
        asset_gets.load(Ordering::Acquire),
        receipts.load(Ordering::Acquire),
        wrong_lease_gets.load(Ordering::Acquire),
    );
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("pass the built local-renderer.js path")?;
    let javascript = std::fs::read(path)?;
    let listener = bind_owned_local_server("127.0.0.1:0".parse()?)?;
    let bound = listener.local_addr()?;
    let origin = LoopbackOrigin::from_socket_addr(bound)?;
    let preview_origin =
        HttpFrameOrigin::from_localhost_subdomain("p-alpha.preview.localhost", bound.port())?;
    let preview_host = format!("p-alpha.preview.localhost:{}", bound.port());
    let main_host = bound.to_string();
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
        std::thread::spawn(move || {
            serve(
                server_socket,
                javascript,
                window,
                stopped,
                passed,
                calls,
                preview_host,
                preview_origin.as_str().to_owned(),
                main_host,
            )
        })
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
    let no_grant = std::env::var_os("WEBUI_NATIVE_IPC_NO_GRANT").is_some();
    // The grant is not an IPC admission and remains live through native close.
    #[cfg(not(target_os = "linux"))]
    let _preview_grant = if no_grant {
        None // Test-only red control: the identical renderer must not load the preview.
    } else {
        Some(frame.frame_policy().allow_unprivileged_origin(
            HttpFrameOrigin::from_localhost_subdomain("p-alpha.preview.localhost", bound.port())?,
        )?)
    };
    window
        .set(frame.window_handle().clone())
        .map_err(|_| "duplicate native window")?;
    {
        let window = Arc::clone(&window);
        let reported = Arc::clone(&passed);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(if no_grant { 20 } else { 45 }));
            if !reported.load(Ordering::Acquire) {
                eprintln!("LOCAL_IPC_FAILURE fixture deadline elapsed");
                if let Some(window) = window.get() {
                    let _ = window.request_close();
                }
            }
        });
    }
    let reported = Arc::clone(&passed);
    let counted = Arc::clone(&calls);
    frame.on_event(move |event| {
        if matches!(event, DesktopEvent::WindowClosed { .. }) {
            if !reported.load(Ordering::Acquire) || counted.load(Ordering::Acquire) != 2 {
                eprintln!("LOCAL_IPC_FAILURE window closed without two authenticated document RPCs");
                std::process::exit(2);
            }
            println!("LOCAL_IPC_RESULT source=owned native_carrier=1 documents=2 subframe_denied=true preview_receipt={PREVIEW_RECEIPTS_REQUIRED} http_ipc_frames=0");
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
    println!("LOCAL_IPC_RESULT source=owned native_carrier=1 documents=2 subframe_denied=true preview_receipt={PREVIEW_RECEIPTS_REQUIRED} http_ipc_frames=0");
    Ok(())
}
