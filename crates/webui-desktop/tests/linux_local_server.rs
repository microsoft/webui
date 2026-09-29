// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Run on a Linux desktop with GTK4, WebKitGTK 6 and a display:
//! cargo test -p microsoft-webui-desktop --features local-server --test linux_local_server -- --nocapture
//! No mock WebView or custom protocol is involved.
//! WebKitGTK cannot identify a navigation action's source frame: a same-origin
//! iframe GET may reach the listener. The response policy prevents its document
//! from committing; it does not prevent that request from leaving the WebView.

#![cfg(all(target_os = "linux", feature = "local-server"))]
#![allow(clippy::disallowed_methods)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use webui_desktop::{
    DesktopApp, DesktopEvent, EventResponse, HostLifetime, LocalServerOptions, LoopbackOrigin,
    WindowCommandError, WindowOptions,
};

const PAGE: &str = r#"<!doctype html><html lang="en"><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Direct HTTP fixture</title><main><h1>Linux direct HTTP</h1>
<form action="/submit" method="post"><input name="value" value="posted"></form>
<iframe src="/subframe"></iframe></main><script>window.cspBypass=true</script>
<script nonce="webui-test">
(async()=>{
  try {
    if(window.cspBypass) throw Error('CSP ignored');
    if(window.webkit?.messageHandlers?.webuiHost) throw Error('native bridge exposed');
    const partial=await fetch('/partial?view=one',{headers:{Accept:'application/json'}});
    if(partial.headers.get('content-type')!=='application/json' ||
       (await partial.json()).view!=='one') throw Error('partial');
    const range=await fetch('/media',{headers:{Range:'bytes=2-5'}});
    if(range.status!==206 || range.headers.get('content-range')!=='bytes 2-5/8' ||
       await range.text()!=='cdef') throw Error('range');
    const stream=await fetch('/stream');
    const reader=stream.body.getReader();
    const first=await reader.read();
    if(first.done || new TextDecoder().decode(first.value)!=='first') throw Error('stream first');
    await fetch('/first-seen',{method:'POST'});
    const second=await reader.read();
    if(second.done || new TextDecoder().decode(second.value)!=='second') throw Error('stream second');
    document.forms[0].submit();
  } catch(error) { await fetch('/report',{method:'POST',body:'FAIL '+String(error)}); }
})();</script></html>"#;
const DONE: &str = r#"<!doctype html><html lang="en"><meta charset="utf-8"><title>Done</title>
<h1>POST + 303 arrived</h1><script nonce="webui-test">
fetch('/report',{method:'POST',body:window.webkit?.messageHandlers?.webuiHost?'FAIL native bridge':'PASS'});
</script></html>"#;

#[derive(Default)]
struct Observed {
    partial: bool,
    range: bool,
    posted: bool,
    first_seen: bool,
    subframe_requests: usize,
    subframe_committed: bool,
    result: Option<String>,
}

// GTK's default main context is process-global. Rust's test harness otherwise
// runs these two GUI journeys concurrently on separate worker threads.
#[test]
fn native_local_server_journeys() -> Result<(), Box<dyn std::error::Error>> {
    direct_http_navigation_and_response_semantics()
        .map_err(|error| std::io::Error::other(format!("direct HTTP journey: {error}")))?;
    synchronous_revocation_closes_with_a_full_command_queue()
        .map_err(|error| std::io::Error::other(format!("owner revocation journey: {error}")))?;
    Ok(())
}

fn direct_http_navigation_and_response_semantics() -> Result<(), Box<dyn std::error::Error>> {
    assert!(
        std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some(),
        "GTK display unavailable: run on a Linux desktop, not headless"
    );
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let origin = LoopbackOrigin::from_socket_addr(listener.local_addr()?)?;
    listener.set_nonblocking(true)?;
    let observations = Arc::new(Mutex::new(Observed::default()));
    let stop = Arc::new(AtomicBool::new(false));
    let server = {
        let observations = Arc::clone(&observations);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let observed = Arc::clone(&observations);
                        std::thread::spawn(move || {
                            if let Err(error) = serve(stream, &observed) {
                                eprintln!("fixture HTTP failure: {error}");
                                observed.lock().unwrap().result =
                                    Some(format!("FAIL HTTP: {error}"));
                            }
                        });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(error) => {
                        observations.lock().unwrap().result = Some(format!("FAIL accept: {error}"));
                        return;
                    }
                }
            }
        })
    };
    let (owner, lifetime) = HostLifetime::new();
    let frame = DesktopApp::from_local_server(LocalServerOptions::new(origin, lifetime))
        .window(WindowOptions {
            title: "Direct HTTP fixture".into(),
            ..WindowOptions::default()
        })
        .build()?;
    let closed = Arc::new(AtomicBool::new(false));
    let closed_event = Arc::clone(&closed);
    let event_trace = Arc::new(Mutex::new(Vec::with_capacity(16)));
    let traced_events = Arc::clone(&event_trace);
    frame.on_event(move |event| {
        if matches!(
            event,
            DesktopEvent::Ready
                | DesktopEvent::NavigationRequested { .. }
                | DesktopEvent::NavigationCompleted { .. }
                | DesktopEvent::WindowCloseRequested { .. }
                | DesktopEvent::WindowClosed { .. }
                | DesktopEvent::Exiting
        ) {
            let mut trace = traced_events.lock().unwrap();
            if trace.len() < 16 {
                trace.push(format!("{event:?}"));
            }
        }
        if matches!(event, DesktopEvent::WindowClosed { .. }) {
            closed_event.store(true, Ordering::Release);
        }
        EventResponse::Continue
    })?;
    let handle = frame.window_handle().clone();
    let observed = Arc::clone(&observations);
    let watchdog = std::thread::spawn(move || {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(20) {
            if observed.lock().unwrap().result.is_some() {
                let _ = handle.request_close();
                return;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        eprintln!("GTK direct HTTP fixture timed out waiting for the browser report");
        // A hung GTK event loop must fail loudly instead of stalling CI.
        std::process::exit(2);
    });
    let result = webui_desktop::run_local_server_frame(frame);
    stop.store(true, Ordering::Release);
    server.join().map_err(|_| "HTTP server panicked")?;
    watchdog.join().map_err(|_| "watchdog panicked")?;
    eprintln!(
        "GTK JOURNEY native_result={result:?} closed={} server_result={:?} events={:?}",
        closed.load(Ordering::Acquire),
        observations.lock().unwrap().result.as_deref(),
        event_trace.lock().unwrap()
    );
    result?;
    assert!(closed.load(Ordering::Acquire), "native window never closed");
    let observed = observations.lock().unwrap();
    assert!(observed.partial, "JSON Accept/query was not preserved");
    assert!(observed.range, "Range request was not delivered directly");
    assert!(observed.posted, "form POST body was not delivered");
    assert!(
        observed.first_seen,
        "stream was buffered before its first chunk reached WebKit"
    );
    eprintln!(
        "SUBFRAME http_requests={} document_committed={}",
        observed.subframe_requests, observed.subframe_committed
    );
    assert!(
        observed.subframe_requests > 0,
        "fixture did not exercise a same-origin subframe HTTP request"
    );
    assert!(
        !observed.subframe_committed,
        "subframe document committed after its HTTP request reached the listener"
    );
    assert_eq!(
        observed.result.as_deref(),
        Some("PASS"),
        "browser fixture failed"
    );
    drop(owner);
    Ok(())
}

fn synchronous_revocation_closes_with_a_full_command_queue(
) -> Result<(), Box<dyn std::error::Error>> {
    assert!(
        std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some(),
        "GTK display unavailable: run on a Linux desktop, not headless"
    );
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let origin = LoopbackOrigin::from_socket_addr(listener.local_addr()?)?;
    let (owner, lifetime) = HostLifetime::new();
    let owner = Arc::new(owner);
    let frame = DesktopApp::from_local_server(
        LocalServerOptions::new(origin, lifetime).initial_path("/deep?from=revocation")?,
    )
    .build()?;
    let commands = frame.window_handle().clone();
    let observed = Arc::new(AtomicBool::new(false));
    let closed = Arc::new(AtomicBool::new(false));
    let failed = Arc::new(AtomicBool::new(false));
    let committed = Arc::new(AtomicBool::new(false));
    let requested = Arc::clone(&observed);
    let closed_event = Arc::clone(&closed);
    let failed_event = Arc::clone(&failed);
    let committed_event = Arc::clone(&committed);
    frame.on_event(move |event| {
        match event {
            DesktopEvent::NavigationRequested { .. } => {
                requested.store(true, Ordering::Release);
                let full = (0..4096).any(|_| {
                    matches!(
                        commands.set_title("queued"),
                        Err(WindowCommandError::QueueFull)
                    )
                });
                if !full || owner.revoke().is_err() {
                    failed_event.store(true, Ordering::Release);
                }
                // Deliberately Continue: adapter must RECHECK the revoked
                // lifetime after this synchronous callback.
            }
            DesktopEvent::NavigationCompleted { .. } => {
                committed_event.store(true, Ordering::Release);
            }
            DesktopEvent::WindowClosed { .. } => {
                closed_event.store(true, Ordering::Release);
            }
            _ => {}
        }
        EventResponse::Continue
    })?;
    let closed_watchdog = Arc::clone(&closed);
    let watchdog = std::thread::spawn(move || {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(10) {
            if closed_watchdog.load(Ordering::Acquire) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        eprintln!("revoked GTK window did not close");
        std::process::exit(2);
    });
    webui_desktop::run_local_server_frame(frame)?;
    watchdog
        .join()
        .map_err(|_| "owner close watchdog panicked")?;
    assert!(
        observed.load(Ordering::Acquire),
        "navigation callback absent"
    );
    assert!(closed.load(Ordering::Acquire), "window not destroyed");
    assert!(
        !failed.load(Ordering::Acquire),
        "full command queue or owner close wake failed"
    );
    assert!(
        !committed.load(Ordering::Acquire),
        "retired navigation committed a document"
    );
    let (connections, request_bytes, deep_gets) = observe_retired_connections(&listener)?;
    eprintln!(
        "REVOKE speculative_connections={connections} request_bytes={request_bytes} deep_gets={deep_gets}"
    );
    assert!(
        deep_gets == 0,
        "an HTTP GET /deep reached the server before revocation could deny navigation"
    );
    Ok(())
}

/// A speculative TCP connect is not an HTTP document request. Inspect each
/// pending connection rather than treating accept() itself as a policy breach.
fn observe_retired_connections(listener: &TcpListener) -> std::io::Result<(usize, usize, usize)> {
    listener.set_nonblocking(true)?;
    let mut connections = 0;
    let mut request_bytes = 0;
    let mut deep_gets = 0;
    let until = Instant::now() + Duration::from_millis(250);
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                connections += 1;
                if connections > 32 {
                    return Err(std::io::Error::other(
                        "too many speculative connections to inspect",
                    ));
                }
                stream.set_read_timeout(Some(Duration::from_millis(100)))?;
                let mut request = [0_u8; 4096];
                let mut length = 0;
                loop {
                    match stream.read(&mut request[length..]) {
                        Ok(0) => break,
                        Ok(count) => {
                            length += count;
                            if length == request.len() {
                                return Err(std::io::Error::other(
                                    "HTTP request exceeds fixture inspection limit",
                                ));
                            }
                        }
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            break;
                        }
                        Err(error) => return Err(error),
                    }
                }
                request_bytes += length;
                if length != 0 && !request[..length].windows(2).any(|pair| pair == b"\r\n") {
                    return Err(std::io::Error::other(
                        "partial HTTP request: cannot certify that no deep GET was sent",
                    ));
                }
                if request[..length]
                    .windows(b"GET /deep?from=revocation".len())
                    .any(|bytes| bytes == b"GET /deep?from=revocation")
                {
                    deep_gets += 1;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= until {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
    Ok((connections, request_bytes, deep_gets))
}

fn serve(mut stream: TcpStream, observed: &Arc<Mutex<Observed>>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut bytes = Vec::with_capacity(2048);
    let end = loop {
        let mut buffer = [0; 2048];
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            return Ok(());
        }
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(index) = bytes.windows(4).position(|slice| slice == b"\r\n\r\n") {
            break index + 4;
        }
        if bytes.len() > 8192 {
            return Err(std::io::Error::other("fixture headers too large"));
        }
    };
    // The header is bounded above. Own it before extending `bytes` with a
    // possibly delayed POST body, which may reallocate the request buffer.
    let headers = String::from_utf8_lossy(&bytes[..end]).into_owned();
    let first = headers.lines().next().unwrap_or_default().to_string();
    let length = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .and_then(|value| value.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    if length > 8192 {
        return Err(std::io::Error::other("fixture body too large"));
    }
    while bytes.len() - end < length {
        let mut buffer = [0; 2048];
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            return Err(std::io::Error::other("fixture body truncated"));
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    let body = &bytes[end..end + length];
    let mut state = observed.lock().unwrap();
    match first.as_str() {
        "GET / HTTP/1.1" => respond(&mut stream, "200 OK", "text/html", PAGE.as_bytes(), "")?,
        "GET /partial?view=one HTTP/1.1" => {
            state.partial = headers
                .to_ascii_lowercase()
                .contains("accept: application/json");
            respond(
                &mut stream,
                "200 OK",
                "application/json",
                br#"{"view":"one"}"#,
                "",
            )?;
        }
        "GET /media HTTP/1.1" => {
            state.range = headers.to_ascii_lowercase().contains("range: bytes=2-5");
            respond(
                &mut stream,
                "206 Partial Content",
                "text/plain",
                b"cdef",
                "Content-Range: bytes 2-5/8\r\nAccept-Ranges: bytes\r\n",
            )?;
        }
        "GET /stream HTTP/1.1" => {
            drop(state);
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nfirst\r\n")?;
            stream.flush()?;
            let start = Instant::now();
            while !observed.lock().unwrap().first_seen {
                if start.elapsed() > Duration::from_secs(5) {
                    return Err(std::io::Error::other("first stream chunk was buffered"));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            stream.write_all(b"6\r\nsecond\r\n0\r\n\r\n")?;
        }
        "POST /first-seen HTTP/1.1" => {
            state.first_seen = true;
            respond(&mut stream, "204 No Content", "text/plain", b"", "")?;
        }
        "POST /submit HTTP/1.1" => {
            state.posted = body == b"value=posted";
            respond(
                &mut stream,
                "303 See Other",
                "text/plain",
                b"",
                "Location: /done\r\n",
            )?;
        }
        "GET /done HTTP/1.1" => respond(&mut stream, "200 OK", "text/html", DONE.as_bytes(), "")?,
        "GET /subframe HTTP/1.1" => {
            state.subframe_requests += 1;
            respond(
                &mut stream,
                "200 OK",
                "text/html",
                b"<script nonce=\"webui-test\">fetch('/subframe-committed',{method:'POST'})</script>",
                "",
            )?;
        }
        "POST /subframe-committed HTTP/1.1" => {
            state.subframe_committed = true;
            respond(&mut stream, "204 No Content", "text/plain", b"", "")?;
        }
        "POST /report HTTP/1.1" => {
            state.result = Some(String::from_utf8_lossy(body).to_string());
            respond(&mut stream, "204 No Content", "text/plain", b"", "")?;
        }
        _ => respond(&mut stream, "404 Not Found", "text/plain", b"", "")?,
    }
    Ok(())
}

fn respond(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    extra: &str,
) -> std::io::Result<()> {
    write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nContent-Security-Policy: default-src 'self'; script-src 'nonce-webui-test'; connect-src 'self'; frame-src 'self'; object-src 'none'\r\n{extra}Connection: close\r\n\r\n", body.len())?;
    stream.write_all(body)
}
