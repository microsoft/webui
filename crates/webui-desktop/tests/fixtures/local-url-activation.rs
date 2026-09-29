// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Manual, controlled AppKit delegate injection; NOT proof of OS URL delivery.
//! On an unlocked macOS desktop:
//! WEBUI_NATIVE_URL_TEST_CONTROLLED=yes cargo run -p microsoft-webui-desktop \
//!   --no-default-features --features local-server --example local-url-activation
#![allow(unsafe_code)]

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSApplication, NSApplicationDelegate, NSApplicationWillFinishLaunchingNotification,
    };
    use objc2_foundation::{NSArray, NSNotification, NSNotificationCenter, NSString, NSURL};
    use std::io::Write;
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use webui_desktop::{
        DesktopApp, DesktopEvent, EventResponse, HostLifetime, LocalServerOptions, LoopbackOrigin,
        WindowId,
    };

    if std::env::var("WEBUI_NATIVE_URL_TEST_CONTROLLED").as_deref() != Ok("yes") {
        return Err("explicit macOS desktop fixture consent required".into());
    }
    let _mtm = MainThreadMarker::new().ok_or("must run on the AppKit main thread")?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let origin = LoopbackOrigin::from_socket_addr(listener.local_addr()?)?;
    let (owner, lifetime) = HostLifetime::new();
    let frame = DesktopApp::from_local_server(LocalServerOptions::new(origin, lifetime)).build()?;
    let (tx, rx) = std::sync::mpsc::channel();
    let main_thread = std::thread::current().id();
    let on_main = Arc::new(AtomicBool::new(false));
    let callback_on_main = Arc::clone(&on_main);
    frame.on_url_activation("testapp", move |activation| {
        if std::thread::current().id() == main_thread {
            callback_on_main.store(true, Ordering::Release);
        }
        let _ = tx.send(activation);
    })?;

    // This block explicitly invokes the AppKit delegate method before its
    // didFinishLaunching callback constructs the local-server window.
    let center = NSNotificationCenter::defaultCenter();
    let cold = Arc::new(AtomicBool::new(false));
    let cold_seen = Arc::clone(&cold);
    let observer = RcBlock::new(move |_: std::ptr::NonNull<NSNotification>| {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        let Some(delegate) = app.delegate() else {
            return;
        };
        let Some(url) = NSURL::URLWithString(&NSString::from_str("testapp://open/cold")) else {
            return;
        };
        let oversized = NSArray::from_slice(&[&*url; 9]);
        delegate.application_openURLs(&app, &oversized);
        let urls = NSArray::from_slice(&[&*url]);
        delegate.application_openURLs(&app, &urls);
        cold_seen.store(true, Ordering::Release);
    });
    // SAFETY: The observer is retained and removed on the main thread. The
    // block contains only Send + Sync state and uses AppKit on its main thread.
    let registration = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(NSApplicationWillFinishLaunchingNotification),
            None,
            None,
            &observer,
        )
    };
    let ready = Arc::new(AtomicBool::new(false));
    let ready_seen = Arc::clone(&ready);
    let close = frame.window_handle().clone();
    frame.on_event(move |event| {
        if matches!(event, DesktopEvent::Ready) {
            if let Some(mtm) = MainThreadMarker::new() {
                let app = NSApplication::sharedApplication(mtm);
                if let Some(delegate) = app.delegate() {
                    if let Some(wrong) =
                        NSURL::URLWithString(&NSString::from_str("other://open/rejected"))
                    {
                        delegate.application_openURLs(&app, &NSArray::from_slice(&[&*wrong]));
                    }
                    if let Some(url) =
                        NSURL::URLWithString(&NSString::from_str("testapp://open/warm"))
                    {
                        let urls = NSArray::from_slice(&[&*url]);
                        delegate.application_openURLs(&app, &urls);
                        ready_seen.store(true, Ordering::Release);
                    }
                }
            }
            let close = close.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(700));
                let _ = close.request_close();
            });
        }
        EventResponse::Continue
    })?;
    let backend = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let page = b"<!doctype html><title>Host-only URL activation fixture</title><h1>Host-only fixture</h1>";
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                page.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(page);
        }
    });
    let result = webui_desktop::run_local_server_frame(frame);
    // SAFETY: This exact observer was installed in the current notification
    // center and is removed after the AppKit run loop returns.
    unsafe {
        center.removeObserver(AsRef::<objc2::runtime::AnyObject>::as_ref(&*registration));
    }
    result?;
    backend.join().map_err(|_| "HTTP fixture thread failed")?;
    if !cold.load(Ordering::Acquire) || !ready.load(Ordering::Acquire) {
        return Err("delegate injection did not run in both lifecycle phases".into());
    }
    let first = rx.recv_timeout(Duration::from_secs(2))?;
    let second = rx.recv_timeout(Duration::from_secs(2))?;
    if first.window_id != WindowId::PRIMARY
        || second.window_id != WindowId::PRIMARY
        || first.url != "testapp://open/cold"
        || second.url != "testapp://open/warm"
        || on_main.load(Ordering::Acquire)
        || rx.try_recv().is_ok()
    {
        return Err("cold/warm delegate injection did not preserve window ownership".into());
    }
    drop(owner);
    println!("APPKIT_DELEGATE_INJECTION_ONLY cold warm primary; OS receipt unproven");
    Ok(())
}
