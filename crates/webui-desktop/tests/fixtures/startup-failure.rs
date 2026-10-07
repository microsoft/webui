// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Manual native-alert fixture on an unlocked macOS or Windows desktop. The
//! temporary, non-sensitive macOS alert is acknowledged by a test-only timer
//! after five seconds, or stopped with an error at 15 seconds. On Windows a
//! person must inspect and acknowledge the actual native dialog.
//! WEBUI_STARTUP_FAILURE_MANUAL=yes cargo run -p microsoft-webui-desktop \
//!   --features startup-failure --example startup-failure
//! On macOS a separately authorized capture can use ALERT_WINDOW_NUMBER during
//! the visible interval; this fixture never requests permission.

#![cfg_attr(target_os = "macos", allow(unsafe_code))]

#[cfg(not(any(target_os = "macos", windows)))]
fn main() {}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use webui_desktop::{present_startup_failure, StartupFailure};

    if std::env::var_os("WEBUI_STARTUP_FAILURE_MANUAL").as_deref()
        != Some(std::ffi::OsStr::new("yes"))
    {
        return Err("native alert acceptance requires an explicitly unlocked desktop".into());
    }
    let failure = StartupFailure::new(
        "Unable to open the application",
        "A required local runtime is unavailable.",
        "Check your installation, then restart the application.",
    )?;
    present_startup_failure(&failure)?;
    println!("NATIVE_STARTUP_ALERT_ACKNOWLEDGED");
    Ok(())
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use objc2_foundation::{NSRunLoop, NSRunLoopCommonModes, NSTimer};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use webui_desktop::{present_startup_failure, StartupFailure};

    if std::env::var_os("WEBUI_STARTUP_FAILURE_MANUAL").as_deref()
        != Some(std::ffi::OsStr::new("yes"))
    {
        return Err("native alert acceptance requires an explicitly unlocked desktop".into());
    }

    let observed = Arc::new(AtomicBool::new(false));
    let clicked = Arc::new(AtomicBool::new(false));
    let observed_for_timer = Arc::clone(&observed);
    let clicked_for_timer = Arc::clone(&clicked);
    let started = Instant::now();
    let timer_block = RcBlock::new(move |_: std::ptr::NonNull<NSTimer>| {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        if started.elapsed() > Duration::from_secs(15) {
            app.stopModalWithCode(-1);
            return;
        }
        let Some(window) = app.modalWindow() else {
            return;
        };
        if !window.isVisible() {
            return;
        }
        if !observed_for_timer.swap(true, Ordering::AcqRel) {
            eprintln!("ALERT_WINDOW_NUMBER={}", window.windowNumber());
        }
        if started.elapsed() < Duration::from_secs(5) {
            return;
        }
        if let Some(button) = window.defaultButtonCell() {
            // SAFETY: The cell is retained from the currently visible modal
            // NSAlert window on AppKit's main thread; fixture only clicks OK.
            unsafe { button.performClick(None) };
            clicked_for_timer.store(true, Ordering::Release);
        }
    });
    // SAFETY: The timer block only captures Send + Sync atomics and an Instant.
    // It acquires AppKit objects exclusively on the main thread.
    let timer = unsafe { NSTimer::timerWithTimeInterval_repeats_block(0.25, true, &timer_block) };
    // SAFETY: The timer and mode are retained for the synchronous modal loop;
    // common modes include AppKit's nested modal event loop.
    unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };

    let failure = StartupFailure::new(
        "Unable to open the application",
        "A required local runtime is unavailable.",
        "Check your installation, then restart the application.",
    )?;
    let result = present_startup_failure(&failure);
    timer.invalidate();
    if !observed.load(Ordering::Acquire) || !clicked.load(Ordering::Acquire) {
        return Err("alert was not observed as a visible native modal with a clickable OK".into());
    }
    result?;
    eprintln!(
        "ALERT_ACKNOWLEDGED elapsed_ms={}",
        started.elapsed().as_millis()
    );
    Ok(())
}
