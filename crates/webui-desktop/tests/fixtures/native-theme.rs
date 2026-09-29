// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Controlled macOS ARM64 fixture, run only with an unlocked desktop:
//! WEBUI_THEME_ARTIFACT_DIR=/tmp/webui-theme cargo run -p microsoft-webui-desktop \
//!   --features native-services --example native-theme
//! Repeat with WEBUI_THEME_NARROW=1. Screenshots include the native titlebar.
//! WEBUI_THEME_REVOKE_QUEUED=1 exercises owner loss between queue admission
//! and the AppKit main-queue drain without loading a second document.
//! No OS preference is changed; only this fixture's window is overridden.

#![cfg_attr(target_os = "macos", allow(unsafe_code))]

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    use block2::RcBlock;
    use objc2::runtime::AnyObject;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSAppearanceCustomization, NSApplication, NSImage};
    use objc2_foundation::{NSError, NSString};
    use objc2_web_kit::WKWebView;
    use webui_desktop::{
        DesktopApp, DesktopEvent, EventResponse, HostLifetime, LocalServerOptions, LoopbackOrigin,
        ThemeMode, WindowOptions,
    };

    const PAGE: &str = "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Theme fixture</title><style>body{font:20px system-ui;background:#fff;color:#171717;margin:2rem}article{max-width:44rem;padding:2rem;border:2px solid currentColor;border-radius:1rem}@media(prefers-color-scheme:dark){body{background:#10171f;color:#f4f7fa}}@media(max-width:480px){body{margin:1rem}article{padding:1rem}}</style><article><h1>Native appearance fixture</h1><p>Window and web content must agree in Light, Dark and System.</p></article>";
    // Test-only query, followed by WebKit's asynchronous native snapshot
    // completion as the paint barrier. rAF is suspended for background or
    // locked desktops and cannot prove WindowServer captured matching pixels.
    // No script is injected into production windows or privileged documents.
    const PAINT_QUERY: &str = "JSON.stringify({dark:matchMedia('(prefers-color-scheme: dark)').matches, background:getComputedStyle(document.body).backgroundColor})";

    #[derive(Clone, serde::Deserialize)]
    struct PagePaint {
        dark: bool,
        background: String,
    }
    struct Sample {
        step: usize,
        window_appearance: String,
        view_appearance: String,
        window_dark: bool,
        view_dark: bool,
        page: PagePaint,
        window_number: isize,
        webkit_tiff: Vec<u8>,
    }

    let output = std::path::PathBuf::from(std::env::var("WEBUI_THEME_ARTIFACT_DIR")?);
    std::fs::create_dir_all(&output)?;
    let narrow = std::env::var_os("WEBUI_THEME_NARROW").is_some();
    let label = if narrow { "narrow" } else { "desktop" };
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let origin = LoopbackOrigin::from_socket_addr(listener.local_addr()?)?;
    listener.set_nonblocking(true)?;
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = Arc::clone(&stop);
    let server = std::thread::spawn(move || {
        while !stopping.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut input = [0_u8; 2048];
                    let _ = stream.read(&mut input);
                    let header = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", PAGE.len());
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(PAGE.as_bytes());
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => eprintln!("fixture server failed: {error}"),
            }
        }
    });
    let (owner, lifetime) = HostLifetime::new();
    let frame = DesktopApp::from_local_server(LocalServerOptions::new(origin, lifetime))
        .window(WindowOptions {
            title: "WebUI native theme fixture".into(),
            width: if narrow { 390 } else { 1024 },
            height: 700,
            ..WindowOptions::default()
        })
        .build()?;
    let services = frame.native_services()?;
    if std::env::var_os("WEBUI_THEME_REVOKE_QUEUED").is_some() {
        let (queued_tx, queued_rx) = mpsc::channel();
        let (probe_tx, probe_rx) = mpsc::channel();
        let revoked = Arc::new(AtomicBool::new(false));
        let revoked_for_events = Arc::clone(&revoked);
        let after_revoke = Arc::new(AtomicUsize::new(0));
        let after_revoke_events = Arc::clone(&after_revoke);
        let event_services = services.clone();
        frame.on_event(move |event| {
            match event {
                DesktopEvent::NavigationCompleted { .. } => {
                    // This callback runs on AppKit's main thread. The queued
                    // theme GCD callback cannot drain until we return.
                    let queued = if let Some(mtm) = MainThreadMarker::new() {
                        let app = NSApplication::sharedApplication(mtm);
                        if let Some(window) =
                            app.mainWindow().or_else(|| app.windows().firstObject())
                        {
                            let before = window.effectiveAppearance().name().to_string();
                            // Request the *opposite* of the native appearance
                            // so an illicit mutation is observable even when
                            // this Mac already uses Light as its OS default.
                            let dark_name = unsafe { objc2_app_kit::NSAppearanceNameDarkAqua };
                            let dark = window
                                .effectiveAppearance()
                                .bestMatchFromAppearancesWithNames(
                                    &objc2_foundation::NSArray::from_slice(&[dark_name]),
                                )
                                .is_some();
                            let queued = event_services.set_theme(if dark {
                                ThemeMode::Light
                            } else {
                                ThemeMode::Dark
                            });
                            if queued.is_ok() {
                                queue_preclose_theme_probe(window, before, probe_tx.clone());
                            }
                            queued
                        } else {
                            Err(webui_desktop::NativeServiceError::ThemeUnavailable)
                        }
                    } else {
                        Err(webui_desktop::NativeServiceError::ThemeUnavailable)
                    };
                    let retirement = owner.revoke();
                    revoked_for_events.store(true, Ordering::Release);
                    let _ = queued_tx.send((queued, retirement));
                }
                DesktopEvent::ThemeChanged { .. } if revoked_for_events.load(Ordering::Acquire) => {
                    after_revoke_events.fetch_add(1, Ordering::AcqRel);
                }
                _ => {}
            }
            EventResponse::Continue
        })?;
        let run = webui_desktop::run_local_server_frame(frame);
        stop.store(true, Ordering::Release);
        server.join().map_err(|_| "fixture server panicked")?;
        let (queued, retirement) = queued_rx.recv_timeout(Duration::from_secs(10))?;
        retirement?;
        let outcome = wait(queued?);
        let (before, after) = probe_rx.recv_timeout(Duration::from_secs(10))?;
        if !matches!(outcome, Err(webui_desktop::NativeServiceError::Closed))
            || before != after
            || after_revoke.load(Ordering::Acquire) != 0
            || !matches!(
                services.current_theme(),
                Err(webui_desktop::NativeServiceError::Closed)
            )
        {
            return Err(format!(
                "revoked queued theme unexpectedly completed: result={outcome:?} native_before={before} native_after={after} theme_events={}",
                after_revoke.load(Ordering::Acquire)
            )
            .into());
        }
        run?;
        println!(
            "THEME_REVOKE_QUEUED=Closed; native_appearance={after}; no post-revocation ThemeChanged"
        );
        return Ok(());
    }
    let (loaded_tx, loaded_rx) = mpsc::channel();
    let (sample_tx, sample_rx) = mpsc::channel();
    let phase = Arc::new(AtomicUsize::new(0));
    let event_phase = Arc::clone(&phase);
    let reload_requested = Arc::new(AtomicBool::new(false));
    let event_reload = Arc::clone(&reload_requested);
    frame.on_event(move |event| {
        if matches!(event, DesktopEvent::NavigationCompleted { .. }) {
            let _ = loaded_tx.send(());
        }
        if matches!(event, DesktopEvent::ThemeChanged { .. })
            || (matches!(event, DesktopEvent::NavigationCompleted { .. })
                && event_phase.load(Ordering::Acquire) == 2)
        {
            let step = event_phase.load(Ordering::Acquire);
            if step == 0 {
                return EventResponse::Continue;
            }
            let reload_on_dark = step == 2
                && matches!(event, DesktopEvent::ThemeChanged { .. })
                && event_reload.swap(false, Ordering::AcqRel);
            println!("THEME_EVENT step={step} reload_on_dark={reload_on_dark}");
            let result = (|| {
                let mtm = MainThreadMarker::new().ok_or("not on AppKit main thread")?;
                let app = NSApplication::sharedApplication(mtm);
                let window = app
                    .mainWindow()
                    .or_else(|| app.windows().firstObject())
                    .ok_or("fixture window unavailable")?;
                let view = window
                    .contentView()
                    .ok_or("content view missing")?
                    .downcast::<WKWebView>()
                    .map_err(|_| "content is not WKWebView")?;
                let native = view.effectiveAppearance();
                let view_appearance = native.name().to_string();
                let window_appearance = window.effectiveAppearance().name().to_string();
                // SAFETY: These names are process-lifetime AppKit constants.
                let dark_name = unsafe { objc2_app_kit::NSAppearanceNameDarkAqua };
                let native_dark = native
                    .bestMatchFromAppearancesWithNames(&objc2_foundation::NSArray::from_slice(&[
                        dark_name,
                    ]))
                    .is_some();
                let window_dark = window
                    .effectiveAppearance()
                    .bestMatchFromAppearancesWithNames(&objc2_foundation::NSArray::from_slice(&[
                        dark_name,
                    ]))
                    .is_some();
                if native_dark != window_dark {
                    return Err("native window and webview appearances disagree");
                }
                let window_number = window.windowNumber();
                let sent = sample_tx.clone();
                let reload_view = view.clone();
                let snapshot_view = view.clone();
                let snapshot_window = window;
                let callback = RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
                    if !error.is_null() {
                        let _ =
                            sent.send(Err(format!("WebKit matchMedia query failed: {:?}", error)));
                        return;
                    }
                    // SAFETY: This controlled query returns JSON in an NSString.
                    let page = (unsafe { value.as_ref() })
                        .and_then(|value| value.downcast_ref::<NSString>())
                        .map(ToString::to_string)
                        .ok_or_else(|| "WebKit paint barrier returned no JSON".to_string())
                        .and_then(|json| serde_json::from_str::<PagePaint>(&json).map_err(|error| error.to_string()));
                    let page = match page {
                        Ok(page) => page,
                        Err(error) => {
                            let _ = sent.send(Err(error));
                            return;
                        }
                    };
                    let snapshot_sent = sent.clone();
                    let snapshot_window = snapshot_window.clone();
                    let painted_view = snapshot_view.clone();
                    let reload_view = reload_view.clone();
                    let snapshot_window_name = window_appearance.clone();
                    let snapshot_view_name = view_appearance.clone();
                    let snapshot_done = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                        if !error.is_null() {
                            let _ = snapshot_sent.send(Err(format!("WKWebView paint snapshot failed: {:?}", error)));
                            return;
                        }
                        // SAFETY: WebKit owns this image during its callback; copy
                        // bounded TIFF bytes before returning to the event loop.
                        let webkit_tiff = (unsafe { image.as_ref() })
                            .and_then(NSImage::TIFFRepresentation)
                            .map(|data| data.to_vec())
                            .filter(|data| !data.is_empty() && data.len() <= 32 * 1024 * 1024);
                        let Some(webkit_tiff) = webkit_tiff else {
                            let _ = snapshot_sent.send(Err("WKWebView paint snapshot missing or too large".into()));
                            return;
                        };
                        snapshot_window.displayIfNeeded();
                        let painted_window_name = snapshot_window.effectiveAppearance().name().to_string();
                        let painted_view_name = painted_view.effectiveAppearance().name().to_string();
                        if painted_window_name != snapshot_window_name
                            || painted_view_name != snapshot_view_name
                        {
                            let _ = snapshot_sent.send(Err(format!(
                                "effective native appearance changed during WebKit paint: window={painted_window_name} view={painted_view_name}"
                            )));
                            return;
                        }
                        println!(
                            "PAINT_BARRIER step={step} window={snapshot_window_name} view={snapshot_view_name} native_window_dark={window_dark} native_view_dark={native_dark} match_media_dark={} background={} wk_tiff_bytes={}",
                            page.dark, page.background, webkit_tiff.len(),
                        );
                        let _ = snapshot_sent.send(Ok(Sample {
                            step,
                            window_appearance: snapshot_window_name.clone(),
                            view_appearance: snapshot_view_name.clone(),
                            window_dark,
                            view_dark: native_dark,
                            page: page.clone(),
                            window_number,
                            webkit_tiff,
                        }));
                        if reload_on_dark {
                            // SAFETY: Only this controlled fixture reloads its own page.
                            let _ = unsafe { reload_view.reload() };
                        }
                    });
                    // SAFETY: This public WebKit snapshot API runs after the
                    // test-only document paint barrier; no production code uses it.
                    unsafe {
                        let _: () = objc2::msg_send![
                            &*snapshot_view,
                            takeSnapshotWithConfiguration: Option::<&AnyObject>::None,
                            completionHandler: &*snapshot_done
                        ];
                    }
                });
                // SAFETY: The controlled fixture reads only media and CSS;
                // WebKit's native snapshot callback supplies the paint barrier.
                unsafe {
                    view.evaluateJavaScript_completionHandler(
                        &NSString::from_str(PAINT_QUERY),
                        Some(&callback),
                    );
                }
                println!("PAINT_BARRIER_REQUEST step={step}");
                Ok::<_, &'static str>(())
            })();
            if let Err(error) = result {
                let _ = sample_tx.send(Err(error.into()));
            }
        }
        EventResponse::Continue
    })?;
    let worker = std::thread::spawn(move || -> Result<(), String> {
        let result = (|| {
            loaded_rx
                .recv_timeout(Duration::from_secs(15))
                .map_err(|error| error.to_string())?;
            let run_id = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_nanos();
            for (step, mode, name) in [
                (1, ThemeMode::Light, "light"),
                (2, ThemeMode::Dark, "dark"),
                (3, ThemeMode::System, "system"),
            ] {
                phase.store(step, Ordering::Release);
                println!("THEME_REQUEST step={step} mode={mode:?}");
                let operation = services
                    .set_theme(mode)
                    .map_err(|error| error.to_string())?;
                if !matches!(
                    services.set_theme(mode),
                    Err(webui_desktop::NativeServiceError::ThemeBusy)
                ) {
                    return Err("theme queue accepted concurrent work for one window".into());
                }
                let state = wait(operation).map_err(|error| error.to_string())?;
                println!("THEME_ACK step={step} state={state:?}");
                let sample =
                    sample_rx
                        .recv_timeout(Duration::from_secs(10))
                        .map_err(|error| {
                            format!("paint barrier did not complete at {name}: {error}")
                        })??;
                if sample.step != step
                    || state.mode != mode
                    || state.dark != sample.view_dark
                    || sample.window_dark != sample.view_dark
                    || sample.view_dark != sample.page.dark
                    || (mode == ThemeMode::Dark && !sample.view_dark)
                    || (mode == ThemeMode::Light && sample.view_dark)
                    || services
                        .current_theme()
                        .map_err(|error| error.to_string())?
                        != state
                {
                    return Err(format!(
                        "theme mismatch at {name}: state={state:?} window={} view={} matchMedia={} step={}",
                        sample.window_appearance, sample.view_appearance, sample.page.dark, sample.step
                    ));
                }
                let expected_background = if state.dark {
                    "rgb(16, 23, 31)"
                } else {
                    "rgb(255, 255, 255)"
                };
                if sample.page.background != expected_background {
                    return Err(format!(
                        "computed background not painted for {name}: {}",
                        sample.page.background
                    ));
                }
                let stem = format!("native-theme-v2-{name}-{label}-{run_id}");
                let wk_path = output.join(format!("{stem}-webkit-candidate.tiff"));
                std::fs::write(&wk_path, &sample.webkit_tiff).map_err(|error| error.to_string())?;
                let (_, wk_body) = captured_pixels(&wk_path)?;
                if !pixel_matches(wk_body, state.dark) {
                    return Err(format!(
                        "WebKit snapshot stale for {name}: expected dark={} body={wk_body:?}; inspect {}",
                        state.dark,
                        wk_path.display()
                    ));
                }
                let path = output.join(format!("{stem}-candidate.png"));
                let status = std::process::Command::new("/usr/sbin/screencapture")
                    .args(["-x", "-l", &sample.window_number.to_string()])
                    .arg(&path)
                    .status()
                    .map_err(|error| error.to_string())?;
                if !status.success() || !path.metadata().is_ok_and(|metadata| metadata.len() > 0) {
                    return Err(format!("native window capture failed: {}", path.display()));
                }
                let (bar, body) = captured_pixels(&path)?;
                if services
                    .current_theme()
                    .map_err(|error| error.to_string())?
                    != state
                {
                    return Err(format!(
                        "native appearance changed while capturing {}; unqualified candidate {}",
                        name,
                        path.display()
                    ));
                }
                println!(
                    "CAPTURE_PIXELS step={step} name={name} native_dark={} match_media_dark={} window_appearance={} wk_appearance={} webkit_body={wk_body:?} titlebar={bar:?} body={body:?}",
                    state.dark, sample.page.dark, sample.window_appearance, sample.view_appearance,
                );
                if !pixel_matches(bar, state.dark) || !pixel_matches(body, state.dark) {
                    return Err(format!(
                        "WindowServer screenshot is stale for {name}: expected dark={} titlebar={bar:?} body={body:?}; unqualified candidate {}",
                        state.dark,
                        path.display()
                    ));
                }
                let verified = output.join(format!("{stem}-verified.png"));
                std::fs::rename(&path, &verified).map_err(|error| error.to_string())?;
                let _ = std::fs::remove_file(&wk_path);
                println!("THEME_VERIFIED_SNAPSHOT={}", verified.display());
                if mode == ThemeMode::Dark {
                    reload_requested.store(true, Ordering::Release);
                    let repeat = services
                        .set_theme(ThemeMode::Dark)
                        .map_err(|error| error.to_string())?;
                    let repeated_state = wait(repeat).map_err(|error| error.to_string())?;
                    let repeated = sample_rx
                        .recv_timeout(Duration::from_secs(10))
                        .map_err(|error| error.to_string())??;
                    if repeated.step != step || repeated_state != state || !repeated.page.dark {
                        return Err("repeat dark request did not retain appearance".into());
                    }
                    loaded_rx
                        .recv_timeout(Duration::from_secs(10))
                        .map_err(|error| error.to_string())?;
                    let reloaded = sample_rx
                        .recv_timeout(Duration::from_secs(10))
                        .map_err(|error| error.to_string())??;
                    if reloaded.step != step
                        || !reloaded.view_dark
                        || !reloaded.page.dark
                        || services
                            .current_theme()
                            .map_err(|error| error.to_string())?
                            != state
                    {
                        return Err("new document did not inherit the current dark snapshot".into());
                    }
                }
            }
            Ok(())
        })();
        owner.revoke().map_err(|error| error.to_string())?;
        result
    });
    let run = webui_desktop::run_local_server_frame(frame);
    stop.store(true, Ordering::Release);
    server.join().map_err(|_| "fixture server panicked")?;
    worker.join().map_err(|_| "fixture worker panicked")??;
    run?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn wait(
    mut request: webui_desktop::ThemeRequest,
) -> Result<webui_desktop::ThemeState, webui_desktop::NativeServiceError> {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll, Waker};
    use std::time::{Duration, Instant};
    let mut cx = Context::from_waker(Waker::noop());
    let until = Instant::now() + Duration::from_secs(12);
    loop {
        if let Poll::Ready(result) = Pin::new(&mut request).poll(&mut cx) {
            return result;
        }
        if Instant::now() >= until {
            return Err(webui_desktop::NativeServiceError::ThemeTimeout);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(target_os = "macos")]
fn pixel_matches(rgb: [u8; 3], dark: bool) -> bool {
    if dark {
        rgb.iter().all(|channel| *channel <= 85)
    } else {
        rgb.iter().all(|channel| *channel >= 200)
    }
}

#[cfg(all(test, target_os = "macos"))]
mod paint_evidence_tests {
    use super::pixel_matches;

    #[test]
    fn a_light_label_rejects_dark_titlebar_or_body_pixels() {
        assert!(pixel_matches([255, 255, 255], false));
        assert!(!pixel_matches([38, 39, 40], false));
        assert!(pixel_matches([17, 23, 30], true));
        assert!(!pixel_matches([255, 255, 255], true));
        assert!(!pixel_matches([80, 80, 220], true));
    }
}

#[cfg(target_os = "macos")]
fn captured_pixels(image: &std::path::Path) -> Result<([u8; 3], [u8; 3]), String> {
    let bitmap = image.with_extension("bmp");
    let result = std::process::Command::new("/usr/bin/sips")
        .args(["-s", "format", "bmp"])
        .arg(image)
        .arg("--out")
        .arg(&bitmap)
        .output()
        .map_err(|error| error.to_string())?;
    if !result.status.success() {
        return Err(format!(
            "could not decode screenshot pixels: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    if !bitmap
        .metadata()
        .is_ok_and(|metadata| metadata.len() <= 64 * 1024 * 1024)
    {
        return Err("native bitmap is missing or exceeds 64 MiB".into());
    }
    let bytes = std::fs::read(&bitmap).map_err(|error| error.to_string())?;
    let _ = std::fs::remove_file(&bitmap);
    if bytes.get(..2) != Some(b"BM")
        || read_bitmap_u16(&bytes, 28)? != 32
        || read_bitmap_u32(&bytes, 30)? != 3
        || [54, 58, 62].map(|offset| read_bitmap_u32(&bytes, offset))
            != [Ok(0x00ff0000), Ok(0x0000ff00), Ok(0x000000ff)]
    {
        return Err("unexpected native screenshot BMP pixel encoding".into());
    }
    let width = i32::from_le_bytes(read_bitmap_array::<4>(&bytes, 18)?);
    let height = i32::from_le_bytes(read_bitmap_array::<4>(&bytes, 22)?);
    if !(100..=8192).contains(&width) || height == 0 || height.unsigned_abs() > 8192 {
        return Err("native screenshot BMP dimensions are invalid".into());
    }
    let width = usize::try_from(width).map_err(|error| error.to_string())?;
    let height_abs = usize::try_from(height.unsigned_abs()).map_err(|error| error.to_string())?;
    let offset =
        usize::try_from(read_bitmap_u32(&bytes, 10)?).map_err(|error| error.to_string())?;
    let pixel = |y: usize| -> Result<[u8; 3], String> {
        let row = if height < 0 { y } else { height_abs - 1 - y };
        let index = offset + (row * width + width / 2) * 4;
        let bgra = bytes
            .get(index..index + 4)
            .ok_or_else(|| "native screenshot BMP pixels are truncated".to_string())?;
        Ok([bgra[2], bgra[1], bgra[0]])
    };
    Ok((pixel(55)?, pixel(height_abs / 2)?))
}

#[cfg(target_os = "macos")]
fn read_bitmap_array<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N], String> {
    bytes
        .get(offset..offset + N)
        .ok_or_else(|| "native screenshot BMP header is truncated".to_string())?
        .try_into()
        .map_err(|error: std::array::TryFromSliceError| error.to_string())
}

#[cfg(target_os = "macos")]
fn read_bitmap_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    Ok(u16::from_le_bytes(read_bitmap_array(bytes, offset)?))
}

#[cfg(target_os = "macos")]
fn read_bitmap_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(read_bitmap_array(bytes, offset)?))
}

#[cfg(target_os = "macos")]
struct QueuedThemeProbe {
    window: objc2::rc::Retained<objc2_app_kit::NSWindow>,
    before: String,
    sender: std::sync::mpsc::Sender<(String, String)>,
}

#[cfg(target_os = "macos")]
#[link(name = "System")]
unsafe extern "C" {
    static _dispatch_main_q: std::ffi::c_void;
    fn dispatch_async_f(
        queue: *mut std::ffi::c_void,
        context: *mut std::ffi::c_void,
        work: unsafe extern "C" fn(*mut std::ffi::c_void),
    );
}

#[cfg(target_os = "macos")]
fn queue_preclose_theme_probe(
    window: objc2::rc::Retained<objc2_app_kit::NSWindow>,
    before: String,
    sender: std::sync::mpsc::Sender<(String, String)>,
) {
    let context = Box::into_raw(Box::new(QueuedThemeProbe {
        window,
        before,
        sender,
    }))
    .cast::<std::ffi::c_void>();
    // SAFETY: This exact retained window is transferred between two turns of
    // the same AppKit main queue, after theme drain but before owner-close
    // drain. Neither the probe nor GCD accesses it on a worker thread.
    unsafe {
        dispatch_async_f(
            std::ptr::addr_of!(_dispatch_main_q).cast_mut(),
            context,
            read_preclose_theme,
        );
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C" fn read_preclose_theme(context: *mut std::ffi::c_void) {
    use objc2_app_kit::NSAppearanceCustomization;
    // SAFETY: queue_preclose_theme_probe transfers one boxed probe to this
    // main-queue callback, which owns it for exactly one native appearance read.
    let probe = unsafe { Box::from_raw(context.cast::<QueuedThemeProbe>()) };
    let QueuedThemeProbe {
        window,
        before,
        sender,
    } = *probe;
    let after = window.effectiveAppearance().name().to_string();
    let _ = sender.send((before, after));
}
