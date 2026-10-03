// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Run explicitly on a macOS ARM64 desktop with a display:
//! cargo run -p microsoft-webui-desktop --features native-services --example native-geometry
//! Only a controlled loopback page is loaded. A worker awaits SDK snapshots;
//! AppKit callbacks independently calculate expected native coordinates. A
//! native button is placed at a measured content-relative anchor (not a CSS
//! anchor) to test screen-to-view placement without guessing zoom factors.

#![cfg_attr(target_os = "macos", allow(unsafe_code))]

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc};
    use std::time::{Duration, Instant};
    use webui_desktop::{
        DesktopApp, DesktopEvent, EventResponse, HostLifetime, LocalServerOptions, LoopbackOrigin,
        NativeServices, ScreenRectPoints, WindowOptions,
    };

    #[derive(Debug)]
    struct Expected {
        rect: ScreenRectPoints,
        window_number: isize,
        backing: f64,
        zoom: f64,
        magnification: f64,
    }

    #[derive(Debug, serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct DomMeasurement {
        rect: DomRect,
        probe_rect: DomRect,
        visual_viewport: VisualViewport,
        device_pixel_ratio: f64,
        inner_width: f64,
        inner_height: f64,
        scroll_x: f64,
        scroll_y: f64,
    }

    #[derive(Debug, serde::Deserialize)]
    struct DomRect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    }

    #[derive(Debug, serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct VisualViewport {
        offset_left: f64,
        offset_top: f64,
        page_left: f64,
        page_top: f64,
        width: f64,
        height: f64,
        scale: f64,
    }

    const TEST_ONLY_DOM_MEASUREMENT: &str = r#"(() => {
        const element = document.querySelector('#test-anchor');
        const probe = document.querySelector('#probe-anchor');
        if (!element || !probe || !window.visualViewport) throw Error('fixture anchor or viewport missing');
        const rect = element.getBoundingClientRect();
        const probeRect = probe.getBoundingClientRect();
        const viewport = window.visualViewport;
        return JSON.stringify({
            rect: {x: rect.x, y: rect.y, width: rect.width, height: rect.height},
            probeRect: {x: probeRect.x, y: probeRect.y, width: probeRect.width, height: probeRect.height},
            visualViewport: {
                offsetLeft: viewport.offsetLeft, offsetTop: viewport.offsetTop,
                pageLeft: viewport.pageLeft, pageTop: viewport.pageTop,
                width: viewport.width, height: viewport.height, scale: viewport.scale
            },
            devicePixelRatio: window.devicePixelRatio,
            innerWidth: window.innerWidth,
            innerHeight: window.innerHeight,
            scrollX: window.scrollX, scrollY: window.scrollY
        });
    })()"#;

    fn sample_document_and_snapshot(
        tx: mpsc::Sender<Result<(String, Vec<u8>), String>>,
        scroll: bool,
    ) -> Result<(), String> {
        use block2::RcBlock;
        use objc2::runtime::AnyObject;
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSApplication, NSImage};
        use objc2_foundation::{NSError, NSString};
        use objc2_web_kit::WKWebView;

        let mtm = MainThreadMarker::new().ok_or("WebKit snapshot requires AppKit main thread")?;
        let app = NSApplication::sharedApplication(mtm);
        let window = app
            .mainWindow()
            .or_else(|| app.windows().firstObject())
            .ok_or("fixture window unavailable")?;
        let view = window
            .contentView()
            .ok_or("fixture view unavailable")?
            .downcast::<WKWebView>()
            .map_err(|_| "fixture content view is not WKWebView")?;
        let snapshot_view = view.clone();
        let script_done = RcBlock::new(move |result: *mut AnyObject, error: *mut NSError| {
            if !error.is_null() {
                // SAFETY: WebKit owns this NSError for the duration of its callback.
                let message = unsafe { &*error }.localizedDescription().to_string();
                let _ = tx.send(Err(format!("fixture DOM measurement failed: {message}")));
                return;
            }
            // SAFETY: WebKit owns the result for this callback. The controlled
            // script returns one JSON string, never a page-selected object.
            let Some(text) = (unsafe { result.as_ref() })
                .and_then(|value| value.downcast_ref::<NSString>())
                .map(ToString::to_string)
            else {
                let _ = tx.send(Err("fixture JS returned no JSON string".into()));
                return;
            };
            let snapshot_tx = tx.clone();
            let snapshot_done = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                if !error.is_null() {
                    // SAFETY: WebKit owns this NSError for the duration of its callback.
                    let message = unsafe { &*error }.localizedDescription().to_string();
                    let _ = snapshot_tx
                        .send(Err(format!("public WKWebView snapshot failed: {message}")));
                    return;
                }
                // SAFETY: WebKit owns the NSImage for this callback; TIFF data
                // is copied before returning and written off the UI thread.
                let bytes = (unsafe { image.as_ref() })
                    .and_then(NSImage::TIFFRepresentation)
                    .map(|data| data.to_vec())
                    .filter(|bytes| !bytes.is_empty() && bytes.len() <= 16 * 1024 * 1024);
                let _ = snapshot_tx.send(bytes.map_or_else(
                    || Err("WKWebView snapshot image missing or exceeds 16 MiB".into()),
                    |bytes| Ok((text.clone(), bytes)),
                ));
            });
            // SAFETY: `takeSnapshotWithConfiguration:completionHandler:` is
            // the public WKWebView API. Passing nil selects its documented
            // default configuration; dynamic binding is confined to this
            // fixture because the optional generated binding is not enabled.
            unsafe {
                let _: () = objc2::msg_send![
                    &*snapshot_view,
                    takeSnapshotWithConfiguration: Option::<&AnyObject>::None,
                    completionHandler: &*snapshot_done
                ];
            }
        });
        // SAFETY: The controlled test script runs only in this fixture's
        // confirmed main document; no production renderer evaluates it.
        let script = if scroll {
            format!(
                "window.scrollTo({{left:48,top:72,behavior:'instant'}});{TEST_ONLY_DOM_MEASUREMENT}"
            )
        } else {
            TEST_ONLY_DOM_MEASUREMENT.to_owned()
        };
        unsafe {
            view.evaluateJavaScript_completionHandler(
                &NSString::from_str(&script),
                Some(&script_done),
            )
        };
        Ok(())
    }

    fn inspect_webkit_snapshot(
        capture: (String, Vec<u8>),
        content: webui_desktop::ContentGeometry,
        overlay: ScreenRectPoints,
        scroll: bool,
        narrow: bool,
    ) -> Result<(), String> {
        let (json, tiff) = capture;
        let dom: DomMeasurement = serde_json::from_str(&json).map_err(|error| error.to_string())?;
        if ![
            dom.rect.x,
            dom.rect.y,
            dom.rect.width,
            dom.rect.height,
            dom.probe_rect.x,
            dom.probe_rect.y,
            dom.probe_rect.width,
            dom.probe_rect.height,
            dom.visual_viewport.offset_left,
            dom.visual_viewport.offset_top,
            dom.visual_viewport.page_left,
            dom.visual_viewport.page_top,
            dom.visual_viewport.width,
            dom.visual_viewport.height,
            dom.visual_viewport.scale,
            dom.device_pixel_ratio,
            dom.inner_width,
            dom.inner_height,
            dom.scroll_x,
            dom.scroll_y,
        ]
        .iter()
        .all(|value| value.is_finite())
        {
            return Err("fixture DOM measurement contains non-finite values".into());
        }
        println!("DOM_MEASUREMENT={dom:?}");
        println!(
            "NATIVE_COMPARISON content={:?} overlay={overlay:?} zoom={} magnification={} backing={}",
            content.screen_rect, content.page_zoom, content.magnification, content.backing_scale
        );
        // A candidate for evaluation against independently painted WK pixels,
        // NOT an SDK conversion. Keep viewport scroll distinct from responsive
        // layout reflow (innerWidth and the fixed anchor's CSS x).
        let candidate_scale = dom.device_pixel_ratio / content.backing_scale;
        let candidate_x = (dom.probe_rect.x - dom.visual_viewport.offset_left) * candidate_scale;
        let candidate_y = (dom.probe_rect.y - dom.visual_viewport.offset_top) * candidate_scale;
        println!(
            "PROBE_CANDIDATE local_points=({candidate_x:.3},{candidate_y:.3},{:.3},{:.3}) css_rect=({:.3},{:.3},{:.3},{:.3}) dpr={} backing={} scroll=({},{}) inner=({},{})",
            dom.probe_rect.width * candidate_scale,
            dom.probe_rect.height * candidate_scale,
            dom.probe_rect.x, dom.probe_rect.y,
            dom.probe_rect.width, dom.probe_rect.height,
            dom.device_pixel_ratio, content.backing_scale,
            dom.scroll_x, dom.scroll_y, dom.inner_width, dom.inner_height,
        );
        let overlay_local_x = overlay.x - content.screen_rect.x;
        let overlay_local_top =
            content.screen_rect.y + content.screen_rect.height - overlay.y - overlay.height;
        println!(
            "ANCHOR_COMPARISON css_rect=({:.3},{:.3},{:.3},{:.3}) native_overlay_local_points=({overlay_local_x:.3},{overlay_local_top:.3},{:.3},{:.3}) visual_viewport=({:.3},{:.3},{:.3},{:.3},scale={:.3})",
            dom.rect.x, dom.rect.y, dom.rect.width, dom.rect.height,
            overlay.width, overlay.height,
            dom.visual_viewport.offset_left, dom.visual_viewport.offset_top,
            dom.visual_viewport.width, dom.visual_viewport.height,
            dom.visual_viewport.scale,
        );
        // This verifies only the deliberately coincident default-zoom fixture
        // placement. It does not generalize to zoom, scrolling, or a player's
        // different DOM/layout model.
        if content.page_zoom == 1.0
            && content.magnification == 1.0
            && [
                dom.rect.x - overlay_local_x,
                dom.rect.y - overlay_local_top,
                dom.rect.width - overlay.width,
                dom.rect.height - overlay.height,
            ]
            .iter()
            .any(|delta| delta.abs() > 0.01)
        {
            return Err("default-zoom fixture DOM/native anchor positions differ".into());
        }
        let session = std::env::var("COPILOT_AGENT_SESSION_ID")
            .map_err(|_| "session ID required for fixture-only imagery")?;
        if !session
            .chars()
            .all(|ch| ch.is_ascii_hexdigit() || ch == '-')
        {
            return Err("invalid session ID for fixture imagery".into());
        }
        let dir =
            std::path::PathBuf::from(std::env::var("HOME").map_err(|error| error.to_string())?)
                .join(".copilot/session-state")
                .join(session)
                .join("files");
        std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
        let zoom_name = if content.page_zoom == 1.0 && content.magnification == 1.0 {
            "default"
        } else {
            "zoom"
        };
        let name = format!(
            "native-geometry-wk-{zoom_name}{}-{}",
            if scroll { "-scroll" } else { "" },
            if narrow { "narrow" } else { "desktop" }
        );
        let tiff_path = dir.join(format!("{name}.tiff"));
        let png_path = dir.join(format!("{name}.png"));
        std::fs::write(&tiff_path, tiff).map_err(|error| error.to_string())?;
        let output = std::process::Command::new("/usr/bin/sips")
            .arg("-s")
            .arg("format")
            .arg("png")
            .arg(&tiff_path)
            .arg("--out")
            .arg(&png_path)
            .output()
            .map_err(|error| error.to_string())?;
        let _ = std::fs::remove_file(&tiff_path);
        if !output.status.success() {
            return Err(format!(
                "sips failed to convert controlled WK snapshot: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        println!("WK_SNAPSHOT={}", png_path.display());
        Ok(())
    }

    fn native_sample(zoom: bool) -> Option<Expected> {
        use objc2::MainThreadMarker;
        use objc2_app_kit::NSApplication;
        use objc2_web_kit::WKWebView;

        let mtm = MainThreadMarker::new()?;
        let app = NSApplication::sharedApplication(mtm);
        let window = app.mainWindow().or_else(|| app.windows().firstObject())?;
        let Some(content) = window.contentView() else {
            eprintln!("native fixture window has no contentView");
            return None;
        };
        let Ok(view) = content.downcast::<WKWebView>() else {
            eprintln!("native fixture contentView is not WKWebView");
            return None;
        };
        if zoom {
            // SAFETY: This fixture callback executes on AppKit's main thread.
            unsafe {
                view.setPageZoom(1.25);
                view.setMagnification(1.1);
            }
        }
        let rect = window.convertRectToScreen(view.convertRect_toView(view.bounds(), None));
        // SAFETY: This fixture callback executes on AppKit's main thread.
        let (page_zoom, magnification) = unsafe { (view.pageZoom(), view.magnification()) };
        Some(Expected {
            rect: ScreenRectPoints {
                x: rect.origin.x,
                y: rect.origin.y,
                width: rect.size.width,
                height: rect.size.height,
            },
            window_number: window.windowNumber(),
            backing: window.backingScaleFactor(),
            zoom: page_zoom,
            magnification,
        })
    }

    fn request_fixture_navigation(path: &'static str) -> Result<(), String> {
        use objc2::MainThreadMarker;
        use objc2_app_kit::NSApplication;
        use objc2_foundation::NSString;
        use objc2_web_kit::WKWebView;

        let mtm =
            MainThreadMarker::new().ok_or("navigation fixture requires AppKit main thread")?;
        let app = NSApplication::sharedApplication(mtm);
        let window = app
            .mainWindow()
            .or_else(|| app.windows().firstObject())
            .ok_or("fixture window unavailable")?;
        let view = window
            .contentView()
            .ok_or("fixture view unavailable")?
            .downcast::<WKWebView>()
            .map_err(|_| "fixture content view is not WKWebView")?;
        // SAFETY: The controlled fixture requests only its own loopback path;
        // no URL or script comes from a renderer or external input.
        unsafe {
            view.evaluateJavaScript_completionHandler(
                &NSString::from_str(&format!("location.assign('{path}')")),
                None,
            );
        }
        Ok(())
    }

    fn snapshot(service: &NativeServices) -> Result<webui_desktop::ContentGeometry, String> {
        use std::future::Future;
        use std::pin::Pin;
        use std::task::{Context, Poll, Waker};
        let mut pending = service
            .content_geometry()
            .map_err(|error| error.to_string())?;
        let mut context = Context::from_waker(Waker::noop());
        let until = Instant::now() + Duration::from_secs(4);
        loop {
            match Pin::new(&mut pending).poll(&mut context) {
                Poll::Ready(Ok(value)) => return Ok(value),
                Poll::Ready(Err(error)) => return Err(error.to_string()),
                Poll::Pending if Instant::now() < until => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Poll::Pending => return Err("UI geometry read timed out".into()),
            }
        }
    }

    fn compare(
        service: &NativeServices,
        expected: &Expected,
    ) -> Result<webui_desktop::ContentGeometry, String> {
        let actual = snapshot(service)?;
        let difference = [
            actual.screen_rect.x - expected.rect.x,
            actual.screen_rect.y - expected.rect.y,
            actual.screen_rect.width - expected.rect.width,
            actual.screen_rect.height - expected.rect.height,
            actual.backing_scale - expected.backing,
            actual.page_zoom - expected.zoom,
            actual.magnification - expected.magnification,
        ];
        println!(
            "NATIVE_GEOMETRY epoch={} revision={} rect={:?} backing={} zoom={} magnification={}",
            actual.document_epoch,
            actual.revision,
            actual.screen_rect,
            actual.backing_scale,
            actual.page_zoom,
            actual.magnification
        );
        if difference.iter().any(|delta| delta.abs() > 0.01) {
            return Err(format!(
                "AppKit rect/scale/zoom mismatch: {actual:?} vs {expected:?}"
            ));
        }
        Ok(actual)
    }

    fn place_native_overlay(
        snapshot: webui_desktop::ContentGeometry,
    ) -> Result<ScreenRectPoints, String> {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSApplication, NSButton};
        use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
        use objc2_web_kit::WKWebView;

        let mtm = MainThreadMarker::new().ok_or("overlay must run on AppKit main thread")?;
        let app = NSApplication::sharedApplication(mtm);
        let window = app
            .mainWindow()
            .or_else(|| app.windows().firstObject())
            .ok_or("fixture window unavailable")?;
        let view = window
            .contentView()
            .ok_or("fixture view unavailable")?
            .downcast::<WKWebView>()
            .map_err(|_| "fixture content view is not WKWebView")?;
        // Anchor is deliberately specified in *native screen points* inside
        // the measured content rect. This does NOT infer CSS pixel coordinates.
        let desired = NSRect::new(
            NSPoint::new(
                snapshot.screen_rect.x + snapshot.screen_rect.width - 172.0,
                snapshot.screen_rect.y + snapshot.screen_rect.height - 58.0,
            ),
            NSSize::new(148.0, 36.0),
        );
        let in_view = view.convertRect_fromView(window.convertRectFromScreen(desired), None);
        let button = NSButton::new(mtm);
        button.setTitle(&NSString::from_str("Native anchor"));
        button.setFrame(in_view);
        view.addSubview(&button);
        let actual = window.convertRectToScreen(button.convertRect_toView(button.bounds(), None));
        if [
            actual.origin.x - desired.origin.x,
            actual.origin.y - desired.origin.y,
            actual.size.width - desired.size.width,
            actual.size.height - desired.size.height,
        ]
        .iter()
        .any(|delta| delta.abs() > 0.01)
        {
            return Err("native overlay did not occupy the measured content anchor".into());
        }
        eprintln!("NATIVE_OVERLAY_AT_SCREEN_RECT={desired:?}");
        Ok(ScreenRectPoints {
            x: desired.origin.x,
            y: desired.origin.y,
            width: desired.size.width,
            height: desired.size.height,
        })
    }

    let slow_navigation = std::env::var_os("WEBUI_GEOMETRY_SLOW_NAV").is_some();
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let origin = LoopbackOrigin::from_socket_addr(listener.local_addr()?)?;
    listener.set_nonblocking(true)?;
    let stopped = Arc::new(AtomicBool::new(false));
    let server_stop = Arc::clone(&stopped);
    let server = std::thread::spawn(move || {
        while !server_stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut buffer = [0; 2048];
                    let size = stream.read(&mut buffer).unwrap_or(0);
                    if slow_navigation
                        && String::from_utf8_lossy(&buffer[..size]).starts_with("GET /slow ")
                    {
                        // Hold the actual provisional navigation long enough
                        // for the host to observe geometry fail closed.
                        std::thread::sleep(Duration::from_secs(2));
                    }
                    eprintln!("geometry fixture HTTP request received");
                    let body = b"<!doctype html><html lang=\"en\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Geometry fixture</title><style>html,body{margin:0;min-width:1600px;min-height:1600px;background:#eef3fb;color:#14233c;font:20px system-ui}main{padding:32px}h1{font-size:28px}#test-anchor{position:fixed;top:22px;right:24px;width:148px;height:36px;box-sizing:border-box;background:#fbbf24;border:2px solid #8a4400;color:#14233c;text-align:center;font:16px/32px system-ui}#probe-anchor{position:absolute;left:180px;top:180px;width:120px;height:48px;background:#ff00ff}</style><main><h1>Controlled native geometry</h1><p>Read-only WKWebView content snapshot</p><div id=\"test-anchor\">DOM anchor</div><div id=\"probe-anchor\"></div></main>";
                    let header = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(body);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => eprintln!("fixture server: {error}"),
            }
        }
    });
    let (owner, lifetime) = HostLifetime::new();
    let narrow = std::env::var_os("WEBUI_GEOMETRY_NARROW").is_some();
    let capture = std::env::var_os("WEBUI_GEOMETRY_SCREENSHOT").is_some();
    let apply_zoom = std::env::var_os("WEBUI_GEOMETRY_DEFAULT_ZOOM").is_none();
    let scroll = std::env::var_os("WEBUI_GEOMETRY_SCROLL").is_some();
    let prevent_navigation = std::env::var_os("WEBUI_GEOMETRY_PREVENT_NAV").is_some();
    let frame = DesktopApp::from_local_server(LocalServerOptions::new(origin, lifetime))
        .window(WindowOptions {
            title: "Controlled native geometry fixture".into(),
            width: if narrow { 390 } else { 720 },
            height: if narrow { 700 } else { 540 },
            ..WindowOptions::default()
        })
        .build()?;
    let service = frame.native_services()?;
    let handle = frame.window_handle().clone();
    let stage = Arc::new(AtomicUsize::new(0));
    let event_stage = Arc::clone(&stage);
    let verified = Arc::new(AtomicBool::new(false));
    let verified_for_events = Arc::clone(&verified);
    let (expected_tx, expected_rx) = mpsc::channel::<Expected>();
    let (blocked_tx, blocked_rx) = mpsc::channel::<()>();
    let (slow_requested_tx, slow_requested_rx) = mpsc::channel::<()>();
    let (slow_completed_tx, slow_completed_rx) = mpsc::channel::<()>();
    let (overlay_tx, overlay_rx) = mpsc::channel::<Result<ScreenRectPoints, String>>();
    let (capture_tx, capture_rx) = mpsc::channel::<Result<(String, Vec<u8>), String>>();
    let measured_overlay = Arc::new(std::sync::Mutex::new(
        None::<webui_desktop::ContentGeometry>,
    ));
    let overlay_for_events = Arc::clone(&measured_overlay);
    frame.on_event(move |event| {
        if prevent_navigation {
            if let DesktopEvent::NavigationRequested { url, .. } = event {
                if url.ends_with("/prevented") {
                    let _ = blocked_tx.send(());
                    return EventResponse::PreventDefault;
                }
            }
        }
        if slow_navigation {
            match event {
                DesktopEvent::NavigationRequested { url, .. } if url.ends_with("/slow") => {
                    let _ = slow_requested_tx.send(());
                }
                DesktopEvent::NavigationCompleted { url, .. } if url.ends_with("/slow") => {
                    let _ = slow_completed_tx.send(());
                }
                _ => {}
            }
        }
        if matches!(
            event,
            DesktopEvent::Ready
                | DesktopEvent::NavigationRequested { .. }
                | DesktopEvent::NavigationCompleted { .. }
                | DesktopEvent::WindowResized { .. }
                | DesktopEvent::WindowMoved { .. }
        ) {
            eprintln!("geometry fixture native event: {event:?}");
        }
        if matches!(event, DesktopEvent::WindowClosed { .. })
            && !verified_for_events.load(Ordering::Acquire)
        {
            eprintln!("native geometry fixture closed before verification");
            std::process::exit(3);
        }
        if matches!(event, DesktopEvent::WindowCloseRequested { .. }) {
            if slow_navigation && event_stage.load(Ordering::Acquire) == 6 {
                event_stage.store(7, Ordering::Release);
                if let Err(error) = request_fixture_navigation("/slow") {
                    eprintln!("could not start slow native navigation: {error}");
                }
                return EventResponse::PreventDefault;
            }
            let measured = overlay_for_events
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            if let Some(measured) = measured {
                let placement = place_native_overlay(measured);
                if placement.is_ok() {
                    if let Err(error) = sample_document_and_snapshot(capture_tx.clone(), scroll) {
                        let _ = capture_tx.send(Err(error));
                    }
                }
                let _ = overlay_tx.send(placement);
                return EventResponse::PreventDefault;
            }
        }
        let desired = match event {
            DesktopEvent::NavigationCompleted { .. }
                if event_stage.load(Ordering::Acquire) == 0 =>
            {
                event_stage.store(1, Ordering::Release);
                Some(false)
            }
            DesktopEvent::WindowResized { .. } if event_stage.load(Ordering::Acquire) == 2 => {
                event_stage.store(3, Ordering::Release);
                Some(apply_zoom)
            }
            DesktopEvent::WindowMoved { .. } if event_stage.load(Ordering::Acquire) == 4 => {
                event_stage.store(5, Ordering::Release);
                Some(false)
            }
            _ => None,
        };
        if let Some(zoom) = desired {
            if let Some(expected) = native_sample(zoom) {
                let _ = expected_tx.send(expected);
            }
            if prevent_navigation && !zoom && event_stage.load(Ordering::Acquire) == 1 {
                if let Err(error) = request_fixture_navigation("/prevented") {
                    eprintln!("could not request prevented navigation: {error}");
                }
            }
        }
        EventResponse::Continue
    })?;
    let worker_stage = Arc::clone(&stage);
    let worker_handle = handle;
    let overlay_for_worker = Arc::clone(&measured_overlay);
    let worker = std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let initial = expected_rx
                .recv_timeout(Duration::from_secs(8))
                .map_err(|error| error.to_string())?;
            let first = compare(&service, &initial)?;
            if prevent_navigation {
                blocked_rx
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(|error| format!("native PreventDefault not observed: {error}"))?;
                let after = compare(&service, &initial)?;
                if after.document_epoch != first.document_epoch {
                    return Err("PreventDefault retired a still-live geometry document".into());
                }
                println!("PREVENTED_NAV_PRESERVED_DOCUMENT={}", after.document_epoch);
            }
            if slow_navigation {
                worker_stage.store(6, Ordering::Release);
                worker_handle
                    .request_close()
                    .map_err(|error| error.to_string())?;
                slow_requested_rx
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(|error| format!("slow WK navigation not requested: {error}"))?;
                let deadline = Instant::now() + Duration::from_secs(1);
                loop {
                    if matches!(
                        service.content_geometry(),
                        Err(webui_desktop::NativeServiceError::GeometryUnavailable)
                    ) {
                        break;
                    }
                    if Instant::now() >= deadline {
                        return Err(
                            "actual provisional navigation did not retire geometry promptly".into(),
                        );
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                println!("PROVISIONAL_GEOMETRY_UNAVAILABLE");
                slow_completed_rx
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(|error| format!("slow WK navigation did not finish: {error}"))?;
                let after = compare(&service, &initial)?;
                if after.document_epoch <= first.document_epoch {
                    return Err("real navigation reused an old geometry epoch".into());
                }
                println!("REAL_NAV_NEW_DOCUMENT_EPOCH={}", after.document_epoch);
                worker_stage.store(1, Ordering::Release);
            }
            println!("NATIVE_GEOMETRY_WINDOW_ID={}", initial.window_number);
            worker_stage.store(2, Ordering::Release);
            worker_handle
                .set_size(
                    if narrow { 420 } else { 810 },
                    if narrow { 710 } else { 570 },
                )
                .map_err(|error| error.to_string())?;
            let resized = expected_rx
                .recv_timeout(Duration::from_secs(5))
                .map_err(|error| error.to_string())?;
            let _ = compare(&service, &resized)?;
            worker_stage.store(4, Ordering::Release);
            worker_handle.center().map_err(|error| error.to_string())?;
            let moved = expected_rx
                .recv_timeout(Duration::from_secs(5))
                .map_err(|error| error.to_string())?;
            let measured = compare(&service, &moved)?;
            if resized.backing != moved.backing {
                println!(
                    "DISPLAY_SCALE_CHANGED {} -> {}",
                    resized.backing, moved.backing
                );
            } else {
                println!("DISPLAY_SCALE_OBSERVED {}", moved.backing);
            }
            *overlay_for_worker
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(measured);
            worker_handle
                .request_close()
                .map_err(|error| error.to_string())?;
            let overlay = overlay_rx
                .recv_timeout(Duration::from_secs(5))
                .map_err(|error| error.to_string())??;
            let capture = capture_rx
                .recv_timeout(Duration::from_secs(8))
                .map_err(|error| error.to_string())??;
            inspect_webkit_snapshot(capture, measured, overlay, scroll, narrow)?;
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("native geometry fixture failed: {error}");
            std::process::exit(2);
        }
        verified.store(true, Ordering::Release);
        println!("NATIVE_GEOMETRY_PASS");
        let _ = std::io::stdout().flush();
        if capture {
            std::thread::sleep(Duration::from_secs(60));
        }
        let _ = worker_handle.request_close();
    });
    let result = webui_desktop::run_local_server_frame(frame);
    stopped.store(true, Ordering::Release);
    worker.join().map_err(|_| "geometry worker panicked")?;
    server.join().map_err(|_| "geometry server panicked")?;
    drop(owner);
    result?;
    Ok(())
}
