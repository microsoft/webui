// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Explicit MANUAL actual-native macOS ARM64 acceptance fixture. Only a
//! controlled temporary directory is displayed; a real user must cancel the
//! first NSOpenPanel, then select its sole child folder in the second.
//! WEBUI_NATIVE_PICKER_MANUAL=yes cargo run -p microsoft-webui-desktop \
//!   --features native-picker --example native-directory

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};
    use std::task::{Context, Poll, Waker};
    use std::time::{Duration, Instant};
    use webui_desktop::{
        DesktopApp, DesktopEvent, DirectoryPick, DirectoryPickerOptions, DirectorySelection,
        EventResponse, HostLifetime, LocalServerOptions, LoopbackOrigin, NativeServiceError,
        WindowOptions,
    };

    fn wait(mut request: DirectoryPick) -> Result<DirectorySelection, String> {
        use std::future::Future;
        use std::pin::Pin;
        let mut context = Context::from_waker(Waker::noop());
        let until = Instant::now() + Duration::from_secs(90);
        loop {
            match Pin::new(&mut request).poll(&mut context) {
                Poll::Ready(result) => return result.map_err(|error| error.to_string()),
                Poll::Pending if Instant::now() < until => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Poll::Pending => {
                    return Err("native picker did not complete within fixture limit".into())
                }
            }
        }
    }

    if std::env::var_os("WEBUI_NATIVE_PICKER_MANUAL").as_deref()
        != Some(std::ffi::OsStr::new("yes"))
    {
        return Err("manual native picker acceptance must be explicitly enabled".into());
    }
    let controlled = tempfile::tempdir()?;
    let initial = controlled.path().canonicalize()?;
    let expected = initial.join("selected-folder");
    std::fs::create_dir(&expected)?;
    eprintln!("CONTROLLED_PICKER_DIRECTORY={}", expected.display());
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let origin = LoopbackOrigin::from_socket_addr(listener.local_addr()?)?;
    listener.set_nonblocking(true)?;
    let stopped = Arc::new(AtomicBool::new(false));
    let server_stopped = Arc::clone(&stopped);
    let server = std::thread::spawn(move || {
        while !server_stopped.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut buffer = [0; 2048];
                    let _ = stream.read(&mut buffer);
                    let body = b"<!doctype html><html lang=\"en\"><title>Controlled picker fixture</title><main>Temporary directory picker</main>";
                    let header = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(body);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => eprintln!("controlled picker HTTP server: {error}"),
            }
        }
    });
    let (owner, lifetime) = HostLifetime::new();
    let frame = DesktopApp::from_local_server(LocalServerOptions::new(origin, lifetime))
        .window(WindowOptions {
            title: "Controlled directory picker fixture".into(),
            ..WindowOptions::default()
        })
        .build()?;
    let services = frame.native_services()?;
    let handle = frame.window_handle().clone();
    let (ready_tx, ready_rx) = mpsc::channel();
    frame.on_event(move |event| {
        if matches!(event, DesktopEvent::NavigationCompleted { .. }) {
            let _ = ready_tx.send(());
        }
        EventResponse::Continue
    })?;

    let worker = std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            ready_rx
                .recv_timeout(Duration::from_secs(5))
                .map_err(|error| error.to_string())?;
            let options = DirectoryPickerOptions::new()
                .title("Controlled directory selection")
                .map_err(|error| error.to_string())?
                .initial_directory(initial)
                .map_err(|error| error.to_string())?;
            let cancel = services
                .pick_directory(options.clone())
                .map_err(|error| error.to_string())?;
            if !matches!(
                services.pick_directory(options.clone()),
                Err(NativeServiceError::PickerBusy)
            ) {
                return Err("one window admitted two simultaneous pickers".into());
            }
            if wait(cancel)? != DirectorySelection::Cancelled {
                return Err("native cancel was not reported as user cancellation".into());
            }
            println!("PICKER_CANCELLED_BY_ACTUAL_PANEL");
            let selection = services
                .pick_directory(options)
                .map_err(|error| error.to_string())?;
            match wait(selection)? {
                DirectorySelection::Selected(directory) if directory == expected => {
                    println!("PICKER_SELECTED_CONTROLLED_DIRECTORY");
                }
                other => return Err(format!("unexpected folder picker result: {other:?}")),
            }
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("native directory fixture failed: {error}");
            std::process::exit(2);
        }
        println!("NATIVE_DIRECTORY_SELECTION_PASS");
        let _ = std::io::stdout().flush();
        let _ = handle.request_close();
    });
    let result = webui_desktop::run_local_server_frame(frame);
    stopped.store(true, Ordering::Release);
    worker.join().map_err(|_| "picker worker panicked")?;
    server.join().map_err(|_| "picker server panicked")?;
    drop(owner);
    result?;
    Ok(())
}
