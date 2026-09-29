// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Explicit macOS arm64 acceptance: a sandboxed exact-grant preview must
//! prove HTTP completion, iframe load, MessageChannel nonce, and painted
//! marker before the host takes twelve public WK snapshots.
//!
//! WEBUI_CAPTURE_ARTIFACT_DIR=<scratch> cargo run -p microsoft-webui-desktop \
//!   --features native-services --example native-capture
//! WEBUI_CAPTURE_NARROW=1 WEBUI_CAPTURE_ARTIFACT_DIR=<scratch> cargo run ...

#![cfg_attr(target_os = "macos", allow(unsafe_code))]

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::future::Future;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::{Path, PathBuf};
    use std::pin::Pin;
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Mutex};
    use std::task::{Context, Poll, Wake, Waker};
    use std::time::{Duration, Instant};

    use block2::RcBlock;
    use objc2::runtime::AnyObject;
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use objc2_foundation::{NSError, NSString};
    use objc2_web_kit::WKWebView;
    use webui_desktop::{
        CaptureError, CaptureOptions, CapturedContent, DesktopApp, DesktopEvent, EventResponse,
        HostLifetime, HttpFrameOrigin, LocalServerOptions, LoopbackOrigin, NativeServices,
        WindowHandle, WindowOptions, MAX_WEB_CAPTURE_CHUNK_BYTES,
    };

    const PARENT_JS: &str = include_str!("native-capture-parent.js");
    const CHILD_JS: &str = include_str!("native-capture-child.js");
    const PREVIEW_HTML: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><style>html,body{margin:0;background:#12ee35}#marker{width:100px;height:80px;background:#ff9a00}</style><script defer src=\"/child.js\"></script></head><body><div id=\"marker\"></div></body></html>";
    const PROBE: &str = "JSON.stringify(window.__captureProof || {error:'parent-script-not-run'})";
    const PIXEL_CHECK: &str = include_str!("native-capture-pixels.py");

    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    struct Script {
        source: &'static str,
        reply: mpsc::Sender<Result<String, String>>,
    }

    fn ui_script(script: Script) -> Result<(), String> {
        let mtm = MainThreadMarker::new().ok_or("fixture is not on the AppKit main thread")?;
        let window = NSApplication::sharedApplication(mtm)
            .mainWindow()
            .or_else(|| {
                NSApplication::sharedApplication(mtm)
                    .windows()
                    .firstObject()
            })
            .ok_or("fixture has no main window")?;
        let view = window
            .contentView()
            .ok_or("fixture has no content view")?
            .downcast::<WKWebView>()
            .map_err(|_| "fixture view is not WKWebView")?;
        let callback = RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
            // SAFETY: WK owns the JS result and error for this callback.
            let result = if let Some(error) = unsafe { error.as_ref() } {
                Err(error.localizedDescription().to_string())
            } else {
                (unsafe { value.as_ref() })
                    .and_then(|value| value.downcast_ref::<NSString>())
                    .map(ToString::to_string)
                    .ok_or_else(|| "fixture JS returned no string".into())
            };
            let _ = script.reply.send(result);
        });
        // SAFETY: Only fixed, controlled fixture scripts run in the main page.
        unsafe {
            view.evaluateJavaScript_completionHandler(
                &NSString::from_str(script.source),
                Some(&callback),
            )
        };
        Ok(())
    }

    fn script(
        handle: &WindowHandle,
        pending: &Mutex<Option<Script>>,
        source: &'static str,
    ) -> Result<String, String> {
        let (tx, rx) = mpsc::channel();
        *pending.lock().map_err(|_| "fixture UI queue poisoned")? =
            Some(Script { source, reply: tx });
        handle.request_close().map_err(|error| error.to_string())?;
        rx.recv_timeout(Duration::from_secs(6))
            .map_err(|error| format!("fixture UI probe timed out: {error}"))?
    }

    fn capture(
        services: &NativeServices,
        waker: &Waker,
    ) -> Result<(CapturedContent, Vec<u8>, Duration), String> {
        let start = Instant::now();
        let mut request = services
            .capture_web_content(CaptureOptions::new())
            .map_err(|error| error.to_string())?;
        if !matches!(
            services.capture_web_content(CaptureOptions::new()),
            Err(CaptureError::Busy)
        ) {
            return Err("concurrent WK snapshot was not rejected as Busy".into());
        }
        let deadline = start + Duration::from_secs(8);
        let content = loop {
            match Pin::new(&mut request).poll(&mut Context::from_waker(waker)) {
                Poll::Ready(Ok(content)) => break content,
                Poll::Ready(Err(error)) => return Err(error.to_string()),
                Poll::Pending if Instant::now() < deadline => {
                    std::thread::park_timeout(deadline.saturating_duration_since(Instant::now()));
                }
                Poll::Pending => return Err("WK snapshot did not complete in eight seconds".into()),
            }
        };
        let mut png = Vec::with_capacity(content.png_bytes);
        while png.len() < content.png_bytes {
            let chunk = services
                .read_captured_content(&content, png.len())
                .map_err(|error| error.to_string())?;
            if chunk.bytes.is_empty()
                || chunk.bytes.len() > MAX_WEB_CAPTURE_CHUNK_BYTES
                || chunk.next_offset != png.len() + chunk.bytes.len()
            {
                return Err("native PNG chunk failed bounded credit or progress".into());
            }
            png.extend_from_slice(&chunk.bytes);
            if chunk.eof != (png.len() == content.png_bytes) {
                return Err("native PNG chunk EOF is inconsistent".into());
            }
        }
        Ok((content, png, start.elapsed()))
    }

    fn check_pixels(
        script_path: &Path,
        path: &Path,
        viewport: &str,
        scroll: bool,
    ) -> Result<String, String> {
        let output = Command::new("python3")
            .arg(script_path)
            .arg(path)
            .arg(viewport)
            .arg(if scroll { "1" } else { "0" })
            .output()
            .map_err(|error| format!("native pixel checker unavailable: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "real WK capture has missing or invalid pixels: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().into())
    }

    fn rss_kib() -> Option<usize> {
        let output = Command::new("/bin/ps")
            .arg("-o")
            .arg("rss=")
            .arg("-p")
            .arg(std::process::id().to_string())
            .output()
            .ok()?;
        String::from_utf8_lossy(&output.stdout).trim().parse().ok()
    }

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let origin = LoopbackOrigin::from_socket_addr(addr)?;
    listener.set_nonblocking(true)?;
    let viewport = if std::env::var_os("WEBUI_CAPTURE_NARROW").is_some() {
        "narrow"
    } else {
        "desktop"
    };
    let artifacts = std::env::var_os("WEBUI_CAPTURE_ARTIFACT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("webui-native-capture-fixture"));
    std::fs::create_dir_all(&artifacts)?;
    // The checker is a local-only fixture tool. No production Python, JS or
    // renderer integration is added by this native service.
    let pixel_script = artifacts.join("native-capture-pixels.py");
    std::fs::write(&pixel_script, PIXEL_CHECK)?;
    let preview_done = Arc::new(AtomicBool::new(false));
    let stopped = Arc::new(AtomicBool::new(false));
    let hits = Arc::new(AtomicUsize::new(0));
    let started = Instant::now();
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Native capture fixture</title><style>html,body{{margin:0;background:#d23456;min-height:1800px}}header{{height:110px;background:#1234ab}}iframe{{display:block;border:0;width:160px;height:120px;margin:10px 0}}</style><script defer src=\"/parent.js\"></script></head><body><header></header><iframe id=\"preview\" title=\"Exact allowed preview\" src=\"http://p-alpha.preview.localhost:{}/preview\"></iframe><div style=\"position:absolute;top:1800px;background:#000\">OFFSCREEN</div></body></html>",
        addr.port()
    );
    let server = {
        let preview_done = Arc::clone(&preview_done);
        let stopped = Arc::clone(&stopped);
        let hits = Arc::clone(&hits);
        std::thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                if hits.fetch_add(1, Ordering::AcqRel) > 64 {
                    eprintln!("capture fixture HTTP connection capacity exceeded");
                    std::process::exit(3);
                }
                let done = Arc::clone(&preview_done);
                let html = html.clone();
                std::thread::spawn(move || {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut buffer = [0_u8; 2048];
                    let size = stream.read(&mut buffer).unwrap_or(0);
                    if size == 0 {
                        return; // Never answer an empty speculative connection.
                    }
                    let text = String::from_utf8_lossy(&buffer[..size]);
                    let first = text.lines().next().unwrap_or("");
                    let preview = first == "GET /preview HTTP/1.1";
                    let expected_host =
                        format!("Host: p-alpha.preview.localhost:{}\r\n", addr.port());
                    if preview && !text.contains(&expected_host) {
                        eprintln!("capture fixture preview Host was not the exact granted host");
                        std::process::exit(3);
                    }
                    if preview {
                        std::thread::sleep(Duration::from_millis(1250));
                    }
                    let (body, kind) = match first {
                        "GET /preview HTTP/1.1" => (PREVIEW_HTML.as_bytes(), "text/html"),
                        "GET /parent.js HTTP/1.1" => (PARENT_JS.as_bytes(), "text/javascript"),
                        "GET /child.js HTTP/1.1" => (CHILD_JS.as_bytes(), "text/javascript"),
                        "GET / HTTP/1.1" | "GET /next HTTP/1.1" => (html.as_bytes(), "text/html"),
                        _ => return, // Reject malformed/unknown paths; never serve main HTML.
                    };
                    let header = format!("HTTP/1.1 200 OK\r\nContent-Type: {kind}; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    if stream.write_all(header.as_bytes()).is_ok()
                        && stream.write_all(body).is_ok()
                        && stream.flush().is_ok()
                    {
                        println!(
                            "HTTP_DONE ms={:.2} path={first} bytes={}",
                            started.elapsed().as_secs_f64() * 1000.0,
                            body.len()
                        );
                        if preview {
                            done.store(true, Ordering::Release);
                        }
                    }
                });
            }
        })
    };
    let (owner, lifetime) = HostLifetime::new();
    let frame = DesktopApp::from_local_server(LocalServerOptions::new(origin, lifetime))
        .window(WindowOptions {
            title: "Controlled native web-content capture".into(),
            width: if viewport == "narrow" { 390 } else { 800 },
            height: if viewport == "narrow" { 700 } else { 600 },
            ..WindowOptions::default()
        })
        .build()?;
    let _grant = frame.frame_policy().allow_unprivileged_origin(
        HttpFrameOrigin::from_localhost_subdomain("p-alpha.preview.localhost", addr.port())?,
    )?;
    let services = frame.native_services()?;
    if !matches!(
        services.capture_web_content(CaptureOptions::new()),
        Err(CaptureError::Unavailable)
    ) {
        return Err("capture admitted before a native main document existed".into());
    }
    let handle = frame.window_handle().clone();
    let pending = Arc::new(Mutex::new(None::<Script>));
    let event_pending = Arc::clone(&pending);
    let allow_close = Arc::new(AtomicBool::new(false));
    let event_allow_close = Arc::clone(&allow_close);
    frame.on_event(move |event| {
        if matches!(event, DesktopEvent::WindowCloseRequested { .. }) {
            if let Some(job) = event_pending.lock().ok().and_then(|mut job| job.take()) {
                let tx = job.reply.clone();
                if let Err(error) = ui_script(job) {
                    let _ = tx.send(Err(error));
                }
                return EventResponse::PreventDefault;
            }
            if !event_allow_close.load(Ordering::Acquire) {
                return EventResponse::PreventDefault;
            }
        }
        EventResponse::Continue
    })?;
    let worker = std::thread::spawn(move || {
        let run = || -> Result<(), String> {
            let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
            let before_rss = rss_kib();
            let deadline = Instant::now() + Duration::from_secs(12);
            let ready: serde_json::Value = loop {
                let text = match script(&handle, &pending, PROBE) {
                    Ok(text) => text,
                    Err(error) if Instant::now() < deadline => {
                        let _ = error;
                        std::thread::sleep(Duration::from_millis(50));
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                let state: serde_json::Value =
                    serde_json::from_str(&text).map_err(|error| error.to_string())?;
                if preview_done.load(Ordering::Acquire)
                    && state["loadCount"].as_u64().unwrap_or(0) >= 1
                    && state["channelReady"] == true
                    && state["readyId"] == "preview-paint-v1"
                    && state["marker"] == "rgb(255, 154, 0)"
                {
                    break state;
                }
                if Instant::now() >= deadline {
                    return Err(format!(
                        "preview had no independently proven readiness: http_done={} state={state}",
                        preview_done.load(Ordering::Acquire)
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            };
            println!(
                "PREVIEW_READY ms={:.2} receipt={} load_count={} mode={} frames={}",
                started.elapsed().as_secs_f64() * 1000.0,
                ready["readyId"],
                ready["loadCount"],
                ready["mode"],
                ready["frames"]
            );
            for index in 0..2 {
                let (content, png, _) = capture(&services, &waker)?;
                let path = artifacts.join(format!("capture-{viewport}-settle-{index}.png"));
                std::fs::write(&path, &png).map_err(|error| error.to_string())?;
                println!("{}", check_pixels(&pixel_script, &path, viewport, false)?);
                services
                    .release_captured_content(&content)
                    .map_err(|error| error.to_string())?;
            }
            let mut previous = None::<CapturedContent>;
            for attempt in 0..12 {
                let (content, png, latency) = capture(&services, &waker)?;
                if let Some(old) = previous.take() {
                    if !matches!(
                        services.read_captured_content(&old, 0),
                        Err(CaptureError::Released)
                    ) {
                        return Err("retake retained a previous PNG".into());
                    }
                }
                let path = artifacts.join(format!("capture-{viewport}-{attempt:02}.png"));
                std::fs::write(&path, &png).map_err(|error| error.to_string())?;
                println!("{}", check_pixels(&pixel_script, &path, viewport, false)?);
                println!(
                    "CAPTURE_OK attempt={attempt} ms={:.2} pixels={}x{} png_bytes={}",
                    latency.as_secs_f64() * 1000.0,
                    content.width,
                    content.height,
                    content.png_bytes
                );
                previous = Some(content);
            }
            let after = script(&handle, &pending, PROBE)?;
            let after: serde_json::Value =
                serde_json::from_str(&after).map_err(|error| error.to_string())?;
            if ready["readyId"] != after["readyId"]
                || ready["readyAt"] != after["readyAt"]
                || ready["loadCount"] != after["loadCount"]
            {
                return Err(
                    "iframe navigation or paint receipt changed during twelve captures".into(),
                );
            }
            let scroll = script(
                &handle,
                &pending,
                "window.scrollTo({left:0,top:72,behavior:'instant'});String(window.scrollY)",
            )?;
            if scroll != "72" {
                return Err(format!("fixture did not scroll the WK viewport: {scroll}"));
            }
            std::thread::sleep(Duration::from_millis(60));
            let (scrolled, png, _) = capture(&services, &waker)?;
            let scroll_path = artifacts.join(format!("capture-{viewport}-scroll.png"));
            std::fs::write(&scroll_path, png).map_err(|error| error.to_string())?;
            println!(
                "{}",
                check_pixels(&pixel_script, &scroll_path, viewport, true)?
            );
            if let Some(old) = previous.take() {
                if !matches!(
                    services.read_captured_content(&old, 0),
                    Err(CaptureError::Released)
                ) {
                    return Err("scroll retake did not release original resource".into());
                }
            }
            if std::env::var_os("WEBUI_CAPTURE_NAV").is_some() {
                let _ = script(
                    &handle,
                    &pending,
                    "location.assign('/next');String('requested')",
                );
                let until = Instant::now() + Duration::from_secs(5);
                loop {
                    if matches!(
                        services.read_captured_content(&scrolled, 0),
                        Err(CaptureError::Released)
                    ) {
                        break;
                    }
                    if Instant::now() > until {
                        return Err("real navigation did not retire prior PNG".into());
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                if !matches!(
                    services.capture_web_content(CaptureOptions::new()),
                    Err(CaptureError::Unavailable)
                ) {
                    return Err(
                        "provisional main navigation did not retire capture admission".into(),
                    );
                }
                let until = Instant::now() + Duration::from_secs(8);
                loop {
                    let state = script(&handle, &pending, PROBE)?;
                    let state: serde_json::Value =
                        serde_json::from_str(&state).map_err(|error| error.to_string())?;
                    if state["readyAt"] != ready["readyAt"]
                        && state["channelReady"] == true
                        && state["loadCount"] == 1
                        && state["marker"] == "rgb(255, 154, 0)"
                    {
                        break;
                    }
                    if Instant::now() > until {
                        return Err(format!(
                            "navigated preview did not establish a fresh child receipt: {state}"
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                let (new_doc, png, _) = capture(&services, &waker)?;
                let path = artifacts.join(format!("capture-{viewport}-new-document.png"));
                std::fs::write(&path, png).map_err(|error| error.to_string())?;
                println!("{}", check_pixels(&pixel_script, &path, viewport, false)?);
                services
                    .release_captured_content(&new_doc)
                    .map_err(|error| error.to_string())?;
                println!("CAPTURE_NAV_EPOCH_PASS");
            }
            let dropped = services
                .capture_web_content(CaptureOptions::new())
                .map_err(|error| error.to_string())?;
            if !matches!(
                services.capture_web_content(CaptureOptions::new()),
                Err(CaptureError::Busy)
            ) {
                return Err("second pending capture was not refused".into());
            }
            drop(dropped);
            if std::env::var_os("WEBUI_CAPTURE_NAV").is_none()
                && !matches!(
                    services.read_captured_content(&scrolled, 0),
                    Err(CaptureError::Released)
                )
            {
                return Err("abandoned retake did not release the scrolled resource".into());
            }
            std::thread::sleep(Duration::from_millis(150));
            let (retained_at_revoke, _, _) = capture(&services, &waker)?;
            owner.revoke().map_err(|error| error.to_string())?;
            if !matches!(
                services.read_captured_content(&retained_at_revoke, 0),
                Err(CaptureError::Closed)
            ) {
                return Err("retired host could still read native PNG bytes".into());
            }
            println!(
                "CAPTURE_LIFECYCLE_PASS rss_before_kib={before_rss:?} rss_after_kib={:?}",
                rss_kib()
            );
            Ok(())
        };
        if let Err(error) = run() {
            eprintln!("native capture fixture failed: {error}");
            std::process::exit(2);
        }
        allow_close.store(true, Ordering::Release);
        let _ = handle.request_close();
    });
    let result = webui_desktop::run_local_server_frame(frame);
    stopped.store(true, Ordering::Release);
    worker
        .join()
        .map_err(|_| "capture fixture worker panicked")?;
    server
        .join()
        .map_err(|_| "capture fixture server panicked")?;
    result?;
    Ok(())
}
