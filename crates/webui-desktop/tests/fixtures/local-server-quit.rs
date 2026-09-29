// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Controlled AppKit Quit selector fixture (not a synthetic WindowHandle close).
//! Run on an unlocked macOS ARM64 desktop:
//! WEBUI_NATIVE_QUIT_TEST_CONTROLLED=yes cargo run -p microsoft-webui-desktop \
//!   --features local-server,application-ipc --example local-server-quit -- veto
//! Repeat with `direct`, `window-close`, `owner-revoke`, `stopped`,
//! `saturated`, or `saturated-veto-stop` (failed command admission, veto,
//! then external AppKit stop). A durable certificate in `target/` proves
//! that the Rust host resumed after AppKit returned. No keyboard event is
//! synthesized, and this fixture does not claim to test physical Cmd+Q.
#![allow(unsafe_code)]

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn mock_backend_child(lock_path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::{Read, Write};
    let mut lock = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_path)?;
    writeln!(lock, "mock backend child pid={}", std::process::id())?;
    lock.sync_all()?;
    println!("BACKEND_READY");
    std::io::stdout().flush()?;
    // The host closes our stdin only after the SDK pin is released. This
    // process is never killed by name or detached from the fixture.
    let mut stdin = std::io::stdin();
    let mut input = [0_u8; 1];
    while stdin.read(&mut input)? != 0 {}
    std::fs::remove_file(lock_path)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use block2::RcBlock;
    use objc2::{sel, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType};
    use objc2_foundation::{NSPoint, NSTimer};
    use std::io::{BufRead, Write};
    use std::net::TcpListener;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use webui_desktop::{
        bind_owned_local_server,
        ipc::{IpcOptions, IpcSchema},
        DesktopApp, DesktopEvent, EventResponse, HostLifetime, IpcRegistry, LocalServerOptions,
        LoopbackOrigin,
    };

    fn schedule_unexpected_stop(fired: Arc<AtomicBool>) {
        let callback = RcBlock::new(move |_: std::ptr::NonNull<NSTimer>| {
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            let app = NSApplication::sharedApplication(mtm);
            fired.store(true, Ordering::SeqCst);
            // Stop AppKit without requesting a window close, then wake
            // nextEvent so its run loop actually returns.
            app.stop(None);
            if let Some(event) =
                NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
                    NSEventType::ApplicationDefined,
                    NSPoint::new(0.0, 0.0),
                    NSEventModifierFlags::empty(),
                    0.0,
                    0,
                    None,
                    0,
                    0,
                    0,
                )
            {
                app.postEvent_atStart(&event, true);
            }
        });
        // SAFETY: Called during Ready on AppKit's UI thread. The one-shot
        // timer captures only a Send + Sync observation.
        unsafe {
            let _ = NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.4, false, &callback);
        }
    }

    static SCHEMA: IpcSchema = IpcSchema {
        name: "webui.test.quit",
        major: 1,
        hash: "f414c04368b1b56d5611c51dcf1db0238183639fdb3423f308396c5ed043baee",
        methods: &[],
    };

    if std::env::var("WEBUI_NATIVE_QUIT_TEST_CONTROLLED").as_deref() != Ok("yes")
        || !cfg!(target_arch = "aarch64")
    {
        return Err("explicit macOS ARM64 desktop consent is required".into());
    }
    let mode = std::env::args().nth(1).ok_or(
        "pass veto, direct, window-close, owner-revoke, stopped, saturated, or saturated-veto-stop",
    )?;
    if mode == "backend-child" {
        let lock_path = std::env::args()
            .nth(2)
            .ok_or("missing mock backend lock path")?;
        return mock_backend_child(std::path::Path::new(&lock_path));
    }
    let veto = match mode.as_str() {
        "veto" => true,
        "direct" | "window-close" | "owner-revoke" | "stopped" | "saturated" | "saturated-veto-stop" => false,
        _ => {
            return Err(
                "pass veto, direct, window-close, owner-revoke, stopped, saturated, or saturated-veto-stop".into(),
            )
        }
    };
    let lock_dir = tempfile::tempdir()?;
    let lock_path = lock_dir.path().join("backend.lock");
    let mut child = Command::new(std::env::current_exe()?)
        .arg("backend-child")
        .arg(&lock_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut ready_line = String::new();
    let child_output = child.stdout.take().ok_or("mock child stdout unavailable")?;
    std::io::BufReader::new(child_output).read_line(&mut ready_line)?;
    if ready_line.trim() != "BACKEND_READY" || !lock_path.is_file() {
        return Err("mock backend child did not acquire its lock".into());
    }
    let listener = bind_owned_local_server("127.0.0.1:0".parse()?)?;
    let address = listener.local_addr()?;
    let origin = LoopbackOrigin::from_socket_addr(address)?;
    let (owner, lifetime) = HostLifetime::new();
    let owner = Arc::new(owner);
    let frame = DesktopApp::from_local_server(LocalServerOptions::new(origin, lifetime))
        .application_ipc(
            &listener,
            IpcRegistry::new(&SCHEMA),
            IpcOptions::for_schema(&SCHEMA),
        )?
        .build()?;
    let commands = frame.window_handle().clone();
    // The only bound socket left before AppKit launches is the SDK's IPC pin.
    // No connections are accepted, so a post-close rebind has no TIME_WAIT
    // dependence. The webview's failed HTTP load is intentional in this test.
    drop(listener);
    assert!(
        TcpListener::bind(address).is_err(),
        "SDK did not pin the listener"
    );

    let ready = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(AtomicUsize::new(0));
    let closed = Arc::new(AtomicUsize::new(0));
    let exiting = Arc::new(AtomicUsize::new(0));
    let second_quit = Arc::new(AtomicBool::new(false));
    let queue_saturated = Arc::new(AtomicBool::new(false));
    let owner_revoked = Arc::new(AtomicBool::new(false));
    let abnormal_stop_fired = Arc::new(AtomicBool::new(false));
    let lifecycle_out_of_order = Arc::new(AtomicBool::new(false));
    let (r, q, c, e, repeated) = (
        Arc::clone(&ready),
        Arc::clone(&requests),
        Arc::clone(&closed),
        Arc::clone(&exiting),
        Arc::clone(&second_quit),
    );
    let test_mode = mode.clone();
    let filled_queue = Arc::clone(&queue_saturated);
    let owner_to_revoke = Arc::clone(&owner);
    let revoked = Arc::clone(&owner_revoked);
    let stop_fired = Arc::clone(&abnormal_stop_fired);
    let out_of_order = Arc::clone(&lifecycle_out_of_order);
    frame.on_event(move |event| {
        match event {
            DesktopEvent::Ready => {
                eprintln!("QUIT_FIXTURE_READY");
                r.fetch_add(1, Ordering::SeqCst);
                let Some(mtm) = MainThreadMarker::new() else {
                    return EventResponse::Continue;
                };
                if std::env::var_os("WEBUI_REQUIRE_WINDOW_MENU").is_some() {
                    let app = NSApplication::sharedApplication(mtm);
                    let root = app.mainMenu();
                    eprintln!("QUIT_FIXTURE_MENU_COUNT={}", root.as_ref().map_or(0, |menu| menu.numberOfItems()));
                    let window_menu = root
                        .filter(|menu| menu.numberOfItems() > 2)
                        .and_then(|menu| menu.itemAtIndex(2))
                        .and_then(|item| item.submenu());
                    eprintln!("QUIT_FIXTURE_WINDOW_MENU={}", window_menu.as_ref().map_or_else(|| "missing".to_string(), |menu| menu.title().to_string()));
                    if window_menu
                        .as_ref()
                        .is_none_or(|menu| menu.title().to_string() != "Window")
                    {
                        eprintln!("QUIT_FIXTURE_MISSING_WINDOW_MENU");
                        std::process::exit(9);
                    }
                    let roles = ["Close Window", "Minimize", "Zoom", "Bring All to Front"];
                    if window_menu.as_ref().is_none_or(|menu| {
                        let mut matched = 0;
                        for index in 0..menu.numberOfItems().min(32) {
                            if menu.itemAtIndex(index).is_some_and(|item| {
                                item.title().to_string() == roles[matched]
                            }) {
                                matched += 1;
                                if matched == roles.len() {
                                    break;
                                }
                            }
                        }
                        matched != roles.len()
                    }) {
                        eprintln!("QUIT_FIXTURE_WINDOW_MENU_ROLES_MISSING");
                        std::process::exit(9);
                    }
                    if test_mode == "window-close" {
                        let Some(window_menu) = window_menu.filter(|menu| menu.numberOfItems() > 0) else {
                            eprintln!("QUIT_FIXTURE_MISSING_WINDOW_CLOSE");
                            std::process::exit(9);
                        };
                        let callback = RcBlock::new(move |_timer: std::ptr::NonNull<NSTimer>| {
                            window_menu.performActionForItemAtIndex(0);
                        });
                        // SAFETY: This fixture schedules a native menu action on
                        // AppKit's main run loop after the window becomes key.
                        unsafe {
                            let _ = NSTimer::scheduledTimerWithTimeInterval_repeats_block(
                                0.4, false, &callback,
                            );
                        }
                        return EventResponse::Continue;
                    }
                }
                if test_mode == "window-close" {
                    if let Err(error) = commands.request_close() {
                        eprintln!("QUIT_FIXTURE_WINDOW_COMMAND_FAILED {error}");
                    }
                    return EventResponse::Continue;
                }
                if test_mode == "owner-revoke" {
                    let owner = Arc::clone(&owner_to_revoke);
                    let revoked = Arc::clone(&revoked);
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(250));
                        match owner.revoke() {
                            Ok(()) => { revoked.store(true, Ordering::SeqCst); }
                            Err(error) => { eprintln!("QUIT_FIXTURE_REVOKE_FAILED {error}"); }
                        }
                    });
                    return EventResponse::Continue;
                }
                if test_mode == "stopped" {
                    schedule_unexpected_stop(Arc::clone(&stop_fired));
                    return EventResponse::Continue;
                }
                let app = NSApplication::sharedApplication(mtm);
                if test_mode == "saturated" || test_mode == "saturated-veto-stop" {
                    // Fill the bounded native command queue before the GCD
                    // drain can run; the Quit close cannot be admitted.
                    let filled = (0..4096).any(|_| commands.set_size(420, 600).is_err());
                    filled_queue.store(filled, Ordering::SeqCst);
                    if test_mode == "saturated-veto-stop" {
                        schedule_unexpected_stop(Arc::clone(&stop_fired));
                    }
                    app.terminate(None);
                    return EventResponse::Continue;
                }
                // A native AppKit terminate: menu action runs after launch,
                // not in applicationDidFinishLaunching's dispatch stack.
                // SAFETY: AppKit owns both target and selector; the one-shot
                // timers run on its main thread during this fixture's run loop.
                unsafe {
                    let _ = NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                        0.4, &app, sel!(terminate:), None, false,
                    );
                    if veto {
                        let _ = NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                            1.2, &app, sel!(terminate:), None, false,
                        );
                    }
                }
            }
            DesktopEvent::WindowCloseRequested { .. } => {
                let attempt = q.fetch_add(1, Ordering::SeqCst) + 1;
                eprintln!("QUIT_FIXTURE_CLOSE_REQUESTED attempt={attempt}");
                if (veto || test_mode == "saturated-veto-stop") && attempt == 1 {
                    // A repeat while the first close is in flight must not
                    // enqueue an independent close or bypass the veto.
                    if veto {
                        if let Some(mtm) = MainThreadMarker::new() {
                            NSApplication::sharedApplication(mtm).terminate(None);
                            repeated.store(true, Ordering::SeqCst);
                        }
                    }
                    return EventResponse::PreventDefault;
                }
            }
            DesktopEvent::WindowClosed { .. } => {
                eprintln!("QUIT_FIXTURE_WINDOW_CLOSED");
                c.fetch_add(1, Ordering::SeqCst);
            }
            DesktopEvent::Exiting => {
                eprintln!("QUIT_FIXTURE_EXITING");
                if c.load(Ordering::SeqCst) != 1 {
                    out_of_order.store(true, Ordering::SeqCst);
                }
                e.fetch_add(1, Ordering::SeqCst);
            }
            _ => {}
        }
        EventResponse::Continue
    })?;
    let watchdog = std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(24));
        // The detached watchdog is terminated when this process returns.
        eprintln!("QUIT_FIXTURE_TIMEOUT: AppKit did not finish native close");
        std::process::exit(7);
    });
    let _ = watchdog.thread().id();
    let result = webui_desktop::run_local_server_frame(frame);
    // A process terminated by NSApplication::terminate cannot create this
    // marker. Persist it before probing the pin or releasing the child.
    let certificate_path = std::env::current_dir()?.join("target").join(format!(
        "webui-local-quit-{}-{}.certificate",
        mode,
        std::process::id()
    ));
    let mut certificate = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&certificate_path)?;
    writeln!(
        certificate,
        "HOST_RESUMED_AFTER_RUN_FRAME mode={mode} pid={}",
        std::process::id()
    )?;
    certificate.sync_all()?;
    if mode == "saturated" || mode == "stopped" || mode == "saturated-veto-stop" {
        assert!(
            (mode != "saturated" && mode != "saturated-veto-stop")
                || queue_saturated.load(Ordering::SeqCst),
            "command queue did not fill"
        );
        let error = result
            .err()
            .ok_or("abnormal native exit reported clean success")?;
        let detail = format!("{error:?}");
        assert!(
            detail.contains(if mode == "stopped" {
                "AppKit loop stopped before WindowClosed"
            } else {
                "command queue is full"
            }),
            "native error did not identify the exercised failure: {detail}"
        );
        eprintln!("QUIT_FIXTURE_EXPECTED_ERROR {detail}");
    } else {
        result?;
    }
    writeln!(
        certificate,
        "FRAME_RESULT={}",
        if matches!(
            mode.as_str(),
            "saturated" | "stopped" | "saturated-veto-stop"
        ) {
            "error"
        } else {
            "ok"
        }
    )?;
    certificate.sync_all()?;
    assert_eq!(ready.load(Ordering::SeqCst), 1);
    assert_eq!(
        requests.load(Ordering::SeqCst),
        if veto {
            2
        } else if mode == "owner-revoke" || mode == "stopped" {
            0
        } else {
            1
        }
    );
    assert!(mode != "owner-revoke" || owner_revoked.load(Ordering::SeqCst));
    assert!(
        (mode != "stopped" && mode != "saturated-veto-stop")
            || abnormal_stop_fired.load(Ordering::SeqCst)
    );
    assert_eq!(closed.load(Ordering::SeqCst), 1);
    assert_eq!(exiting.load(Ordering::SeqCst), 1);
    assert!(!lifecycle_out_of_order.load(Ordering::SeqCst));
    assert!(!veto || second_quit.load(Ordering::SeqCst));
    // The SDK duplicate is gone even while a separate backend process and
    // its lock remain alive. This rebind has no accepted-connection TIME_WAIT.
    let rebound = TcpListener::bind(address)?;
    assert!(
        lock_path.is_file(),
        "mock backend drained before SDK pin release"
    );
    assert!(
        child.try_wait()?.is_none(),
        "mock backend child exited before SDK pin release"
    );
    writeln!(
        certificate,
        "WINDOW_CLOSED=1 EXITING=1 CLOSED_BEFORE_EXITING=true IPC_PIN_RELEASED_REBIND=true MOCK_CHILD_ALIVE=true LOCK_HELD=true"
    )?;
    certificate.sync_all()?;
    println!(
        "QUIT_RESULT mode={} requested={} closed=1 exiting=1 frame_returned=true ipc_pin_released=true mock_child_alive=true",
        mode,
        requests.load(Ordering::SeqCst)
    );
    drop(rebound);
    drop(child.stdin.take());
    let child_status = child.wait()?;
    assert!(child_status.success(), "mock backend child cleanup failed");
    assert!(
        !lock_path.exists(),
        "mock backend lock survived child cleanup"
    );
    writeln!(certificate, "MOCK_CHILD_EXITED=true LOCK_RELEASED=true")?;
    certificate.sync_all()?;
    println!("QUIT_CERTIFICATE={}", certificate_path.display());
    drop(owner);
    Ok(())
}
