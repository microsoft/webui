// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Manual actual-native smoke: a direct GET, same-origin navigation, denied
//! cross-origin navigation and window closure. No protocol or source compiler.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use webui_desktop::{
    DesktopApp, DesktopEvent, EventResponse, HostLifetime, HttpFrameOrigin, LocalServerOptions,
    LoopbackOrigin, WindowOptions,
};

const PAGE: &str = "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>WebUI local-server smoke</title><style>body{font:18px system-ui;margin:0;background:#f4f8ff;color:#14233c}main{max-width:650px;margin:8vh auto;padding:24px}article{padding:28px;border-radius:16px;background:white;box-shadow:0 8px 28px #14233c20}h1{font-size:clamp(28px,5vw,42px)}@media(max-width:600px){main{margin:24px auto;padding:14px}}</style><main><article><h1>Direct loopback page</h1><p>Served over HTTP by the existing local server.</p><p id=\"status\">Opening same-origin route...</p></article></main><script>setTimeout(()=>location.assign('/deep?from=smoke'),900)</script></html>";
const DEEP: &str = "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>WebUI local-server smoke - Deep</title><style>body{font:18px system-ui;margin:0;background:#f4f8ff;color:#14233c}main{max-width:650px;margin:8vh auto;padding:24px}article{padding:28px;border-radius:16px;background:white;box-shadow:0 8px 28px #14233c20}h1{font-size:clamp(28px,5vw,42px)}@media(max-width:600px){main{margin:24px auto;padding:14px}}</style><main><article><h1>Same-origin route loaded</h1><p>Direct HTTP GET, no proxy or second renderer.</p><p>Cross-origin navigation is denied.</p></article></main><iframe hidden src=\"/frame\"></iframe><script>setTimeout(()=>location.assign('/redirect'),1100)</script></html>";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let preview_mode = std::env::var_os("WEBUI_SMOKE_PREVIEW").is_some();
    let navigation_revoke = std::env::var_os("WEBUI_SMOKE_NAV_REVOKE").is_some();
    let drop_owner = std::env::var_os("WEBUI_SMOKE_DROP_OWNER").is_some();
    let revoke_mode = drop_owner || std::env::var_os("WEBUI_SMOKE_REVOKE").is_some();
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let other_listener = TcpListener::bind("127.0.0.1:0")?;
    let other_url = format!("http://{}/", other_listener.local_addr()?);
    let bound = listener.local_addr()?;
    let origin = LoopbackOrigin::from_socket_addr(bound)?;
    let preview_host = format!("p-alpha.preview.localhost:{}", bound.port());
    let stop = Arc::new(AtomicBool::new(false));
    let redirects = Arc::new(AtomicUsize::new(0));
    let server_redirects = Arc::clone(&redirects);
    let frames = Arc::new(AtomicUsize::new(0));
    let server_frames = Arc::clone(&frames);
    let previews = Arc::new(AtomicUsize::new(0));
    let server_previews = Arc::clone(&previews);
    let slow_gets = Arc::new(AtomicUsize::new(0));
    let server_slow_gets = Arc::clone(&slow_gets);
    let deep_gets = Arc::new(AtomicUsize::new(0));
    let server_deep_gets = Arc::clone(&deep_gets);
    let other_hits = Arc::new(AtomicUsize::new(0));
    let other_server_hits = Arc::clone(&other_hits);
    listener.set_nonblocking(true)?;
    other_listener.set_nonblocking(true)?;
    let other_stop = Arc::clone(&stop);
    let other_server = std::thread::spawn(move || {
        while !other_stop.load(Ordering::Acquire) {
            match other_listener.accept() {
                Ok((mut stream, _)) => {
                    other_server_hits.fetch_add(1, Ordering::Relaxed);
                    let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 13\r\nConnection: close\r\n\r\nOther origin!");
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => {
                    eprintln!("other server accept failed: {error}");
                    break;
                }
            }
        }
    });
    let stopping = Arc::clone(&stop);
    let redirect_target = other_url.clone();
    let preview_page = if preview_mode {
        DEEP.replace(
            "<iframe hidden src=\"/frame\"></iframe>",
            &format!("<iframe sandbox=\"allow-scripts allow-same-origin allow-forms allow-downloads\" src=\"http://{preview_host}/preview?lease=alpha\"></iframe>"),
        )
    } else {
        DEEP.to_string()
    };
    let deep_page = if revoke_mode {
        preview_page.replace("'/redirect'", "'/slow'")
    } else {
        preview_page
    };
    let server = std::thread::spawn(move || {
        while !stopping.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut bytes = [0_u8; 2048];
                    if let Ok(size) = stream.read(&mut bytes) {
                        let request = String::from_utf8_lossy(&bytes[..size]);
                        if request.starts_with("GET /frame ") {
                            server_frames.fetch_add(1, Ordering::Relaxed);
                        }
                        if request.starts_with("GET /preview?lease=alpha ") {
                            if !request.contains(&format!("Host: {preview_host}\r\n")) {
                                eprintln!("preview request did not use the exact lease host");
                                std::process::exit(7);
                            }
                            server_previews.fetch_add(1, Ordering::Relaxed);
                            let body = b"<!doctype html><html lang=\"en\"><title>Isolated preview</title><h1>Preview loaded</h1>";
                            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                            let _ = stream.write_all(response.as_bytes());
                            let _ = stream.write_all(body);
                            continue;
                        }
                        if request.starts_with("GET /slow ") {
                            server_slow_gets.fetch_add(1, Ordering::Relaxed);
                            eprintln!("HTTP GET /slow (response held during owner loss)");
                            std::thread::sleep(Duration::from_secs(5));
                            let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", PAGE.len());
                            let _ = stream.write_all(response.as_bytes());
                            let _ = stream.write_all(PAGE.as_bytes());
                            continue;
                        }
                        if request.starts_with("GET /redirect ") {
                            server_redirects.fetch_add(1, Ordering::Relaxed);
                            let response = format!("HTTP/1.1 302 Found\r\nLocation: {redirect_target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                            let _ = stream.write_all(response.as_bytes());
                            continue;
                        }
                        let deep_request = request.starts_with("GET /deep?");
                        if deep_request {
                            server_deep_gets.fetch_add(1, Ordering::Relaxed);
                        }
                        let page = if deep_request {
                            deep_page.as_str()
                        } else {
                            PAGE
                        };
                        println!("HTTP GET {}", if deep_request { "/deep" } else { "/" });
                        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", page.len());
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.write_all(page.as_bytes());
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(error) => {
                    eprintln!("smoke server accept failed: {error}");
                    break;
                }
            }
        }
    });

    let narrow = std::env::var_os("WEBUI_SMOKE_NARROW").is_some();
    let (owner, lifetime) = HostLifetime::new();
    let owner = Arc::new(owner);
    let owner_for_navigation = navigation_revoke.then(|| Arc::clone(&owner));
    let options = LocalServerOptions::new(origin.clone(), lifetime);
    let frame = DesktopApp::from_local_server(options)
        .window(WindowOptions {
            title: "WebUI local-server smoke".to_string(),
            width: if narrow { 390 } else { 1040 },
            height: if narrow { 700 } else { 750 },
            ..WindowOptions::default()
        })
        .build()?;
    let _preview_grant = if preview_mode {
        Some(frame.frame_policy().allow_unprivileged_origin(
            HttpFrameOrigin::from_localhost_subdomain("p-alpha.preview.localhost", bound.port())?,
        )?)
    } else {
        None
    };
    let deep = Arc::new(AtomicUsize::new(0));
    let denied = Arc::new(AtomicUsize::new(0));
    let deep_count = Arc::clone(&deep);
    let denied_count = Arc::clone(&denied);
    let redirect_count = Arc::clone(&redirects);
    let frame_count = Arc::clone(&frames);
    let preview_count = Arc::clone(&previews);
    let other_count = Arc::clone(&other_hits);
    let slow_count = Arc::clone(&slow_gets);
    let deep_get_count = Arc::clone(&deep_gets);
    let navigation_revoke_count = Arc::new(AtomicUsize::new(0));
    let requested_revoke = Arc::clone(&navigation_revoke_count);
    let revoked = Arc::new(AtomicBool::new(false));
    let revoked_for_events = Arc::clone(&revoked);
    let window_closed = Arc::new(AtomicBool::new(false));
    let closed_for_events = Arc::clone(&window_closed);
    let other_url_for_events = other_url;
    frame.on_event(move |event| {
        match event {
            DesktopEvent::NavigationCompleted { url, .. } => {
                println!("COMPLETED {url}");
                if url.starts_with(&other_url_for_events) {
                    eprintln!("unexpected cross-origin document committed");
                    std::process::exit(3);
                }
                if navigation_revoke && url.contains("/deep?") {
                    eprintln!("retired synchronous navigation committed");
                    std::process::exit(5);
                }
                if revoke_mode && url.contains("/slow") {
                    eprintln!("retired in-flight document committed");
                    std::process::exit(5);
                }
                if url.contains("/deep?") {
                    deep_count.fetch_add(1, Ordering::Relaxed);
                }
            }
            DesktopEvent::NavigationRequested { url, .. }
                if url.starts_with(&other_url_for_events) =>
            {
                eprintln!("DENIED {url}");
                denied_count.fetch_add(1, Ordering::Relaxed);
            }
            DesktopEvent::NavigationRequested { url, .. }
                if navigation_revoke && url.contains("/deep?") =>
            {
                requested_revoke.fetch_add(1, Ordering::Relaxed);
                revoked_for_events.store(true, Ordering::Release);
                if let Some(owner) = &owner_for_navigation {
                    if let Err(error) = owner.revoke() {
                        eprintln!("synchronous owner revoke failed: {error}");
                        std::process::exit(6);
                    }
                }
                eprintln!("SYNCHRONOUS_REVOKE {url}; handler returns Continue");
            }
            DesktopEvent::NavigationRequested { url, .. } => eprintln!("REQUESTED {url}"),
            DesktopEvent::WindowClosed { .. } => {
                closed_for_events.store(true, Ordering::Release);
                let deep = deep_count.load(Ordering::Relaxed);
                let denied = denied_count.load(Ordering::Relaxed);
                let redirects = redirect_count.load(Ordering::Relaxed);
                let frames = frame_count.load(Ordering::Relaxed);
                let previews = preview_count.load(Ordering::Relaxed);
                let other_hits = other_count.load(Ordering::Relaxed);
                let slow_gets = slow_count.load(Ordering::Relaxed);
                let deep_gets = deep_get_count.load(Ordering::Relaxed);
                eprintln!("SMOKE deep_get={deep_gets} deep_commit={deep} redirect={redirects} slow_get={slow_gets} revoked={} subframe_get={frames} preview_get={previews} other_origin_get={other_hits} denied_request={denied}", revoked_for_events.load(Ordering::Acquire));
                if (navigation_revoke && (requested_revoke.load(Ordering::Relaxed) != 1
                    || deep_gets != 0 || deep != 0
                    || !revoked_for_events.load(Ordering::Acquire)))
                    || (!navigation_revoke && deep == 0)
                    || frames != 0 || other_hits != 0
                    || (preview_mode && previews == 0)
                    || (!navigation_revoke && revoke_mode && (slow_gets == 0 || !revoked_for_events.load(Ordering::Acquire)))
                    || (!navigation_revoke && !revoke_mode && redirects == 0) {
                    std::process::exit(2);
                }
            }
            _ => {}
        }
        EventResponse::Continue
    })?;
    let retained_owner = if navigation_revoke {
        let closed = Arc::clone(&window_closed);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(12));
            if !closed.load(Ordering::Acquire) {
                eprintln!("synchronous navigation revocation did not close native window");
                std::process::exit(4);
            }
        });
        Some(owner)
    } else if revoke_mode {
        let retired = Arc::clone(&revoked);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(4));
            retired.store(true, Ordering::Release);
            if drop_owner {
                drop(owner);
            } else {
                if let Err(error) = owner.revoke() {
                    eprintln!("native owner close wake failed: {error}");
                    std::process::exit(6);
                }
            }
            eprintln!(
                "OWNER_REVOKED; listener still available: {}",
                std::net::TcpStream::connect(bound).is_ok()
            );
        });
        let closed = Arc::clone(&window_closed);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(12));
            if !closed.load(Ordering::Acquire) {
                eprintln!("owner loss did not close native window");
                std::process::exit(4);
            }
        });
        None
    } else {
        let handle = frame.window_handle().clone();
        let screenshot_time = std::env::var_os("WEBUI_SMOKE_SCREENSHOT").is_some();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(if screenshot_time { 60 } else { 9 }));
            let _ = handle.request_close();
        });
        Some(owner)
    };
    println!("LOCAL_URL={}/", origin.as_str());
    let start = Instant::now();
    let result = webui_desktop::run_local_server_frame(frame);
    drop(retained_owner);
    stop.store(true, Ordering::Release);
    server.join().map_err(|_| "smoke server thread panicked")?;
    other_server
        .join()
        .map_err(|_| "other server thread panicked")?;
    result?;
    println!(
        "SMOKE deep={} denied={} elapsed={:?}",
        deep.load(Ordering::Relaxed),
        denied.load(Ordering::Relaxed),
        start.elapsed()
    );
    if (navigation_revoke
        && (navigation_revoke_count.load(Ordering::Relaxed) != 1
            || deep_gets.load(Ordering::Relaxed) != 0
            || deep.load(Ordering::Relaxed) != 0))
        || (!navigation_revoke && deep.load(Ordering::Relaxed) == 0)
        || (!navigation_revoke && !revoke_mode && redirects.load(Ordering::Relaxed) == 0)
        || (!navigation_revoke && revoke_mode && slow_gets.load(Ordering::Relaxed) == 0)
        || frames.load(Ordering::Relaxed) != 0
        || (preview_mode && previews.load(Ordering::Relaxed) == 0)
        || other_hits.load(Ordering::Relaxed) != 0
    {
        return Err("native direct GET, deep navigation, or cross-origin guard failed".into());
    }
    Ok(())
}
