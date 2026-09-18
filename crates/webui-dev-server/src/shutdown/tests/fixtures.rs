// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::super::{prepare, Control, Error, Mode, CHILD_ENV, JOINED_EXIT_CODE};
use super::support::{command, wait_for, TestResult, FIXTURE_MODE, FIXTURE_OUTPUT};
use crate::{spawn_rebuild_worker, LiveReload};
use actix_web::{web, App, HttpResponse, HttpServer};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

#[test]
#[ignore = "internal subprocess fixture; invoked only by shutdown process tests"]
fn process_entry() {
    let result = run();
    let code = match result {
        Ok(code) => code,
        Err(error) => {
            if let Some(path) = std::env::var_os(FIXTURE_OUTPUT) {
                let _ = fs::write(Path::new(&path).join("fixture-error"), error.to_string());
            }
            91
        }
    };
    // Test harness success (exit 0) must not impersonate the private attestation.
    std::process::exit(code);
}

fn run() -> TestResult<i32> {
    let mode = std::env::var(FIXTURE_MODE)?;
    let output = std::env::var_os(FIXTURE_OUTPUT).ok_or("missing fixture output")?;
    let output = Path::new(&output);
    if mode == "descendant" {
        let mut writer = command("writer", output)?.spawn()?;
        fs::write(
            output.join("descendant.pid"),
            std::process::id().to_string(),
        )?;
        writer.wait()?;
        return Ok(0);
    }
    if mode == "writer" {
        return writer(output);
    }
    if mode == "marker-check" {
        assert!(std::env::var_os(CHILD_ENV).is_none());
        fs::write(output.join("marker-check"), b"absent")?;
        return Ok(0);
    }
    if mode == "prepare" {
        return match prepare(std::num::NonZeroU64::new(2))? {
            Mode::Child(_) => {
                fs::write(output.join("root.ready"), b"child released")?;
                Ok(JOINED_EXIT_CODE)
            }
            Mode::Supervisor(code) => {
                fs::write(output.join("parent.complete"), b"confirmed")?;
                Ok(code)
            }
            Mode::Direct => Err("prepare did not supervise".into()),
        };
    }
    if mode == "direct" {
        assert!(matches!(prepare(None)?, Mode::Direct));
        ctrlc::set_handler(|| {})?;
        fs::write(output.join("direct.complete"), b"no handler or child")?;
        return Ok(0);
    }
    let Mode::Child(control) = prepare(None)? else {
        return Err("fixture was not a supervised child".into());
    };
    assert!(std::env::var_os(CHILD_ENV).is_none());
    if mode == "http" || mode == "http-hung" {
        return http(output, control, mode == "http-hung");
    }
    if matches!(mode.as_str(), "tree" | "root-exits" | "false-attestation") {
        let descendant = command("descendant", output)?.spawn()?;
        wait_for(&output.join("writer.ready"))?;
        // Deliberately owned by the supervisor scope, including root-exit cases.
        drop(descendant);
    }
    fs::write(output.join("root.ready"), b"ready")?;
    match mode.as_str() {
        #[cfg(unix)]
        "quit" => {
            super::super::unix_signals::test_forwarding_and_restoration()?;
            fs::write(
                output.join("quit.complete"),
                b"forwarded twice and restored",
            )?;
            Ok(JOINED_EXIT_CODE)
        }
        "exit-zero" | "root-exits" => Ok(0),
        "false-attestation" => Ok(JOINED_EXIT_CODE),
        "error" => Ok(23),
        "marker" => {
            let status = command("marker-check", output)?.status()?;
            assert!(status.success());
            Ok(JOINED_EXIT_CODE)
        }
        "hung" | "tree" => hang(),
        "graceful" => {
            let mut control = control.0;
            tokio::runtime::Builder::new_current_thread()
                .build()?
                .block_on(async {
                    loop {
                        if *control.borrow_and_update() != super::super::control::State::Running {
                            return Ok::<(), tokio::sync::watch::error::RecvError>(());
                        }
                        control.changed().await?;
                    }
                })?;
            fs::write(output.join("stopping"), b"stop")?;
            wait_for(&output.join("release"))?;
            fs::write(output.join("complete"), b"completed")?;
            Ok(JOINED_EXIT_CODE)
        }
        _ => Err("unknown fixture mode".into()),
    }
}

fn writer(output: &Path) -> TestResult<i32> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(output.join("writes"))?;
    file.write_all(b"x")?;
    fs::write(output.join("writer.ready"), std::process::id().to_string())?;
    loop {
        file.write_all(b"x")?;
        thread::sleep(Duration::from_millis(10));
    }
}

fn hang() -> ! {
    loop {
        thread::park();
    }
}

fn http(output: &Path, control: Control, hung: bool) -> TestResult<i32> {
    let (started, active) = mpsc::channel();
    let (release, continue_work) = mpsc::channel();
    let completed = output.join("complete");
    let builds = Arc::new(AtomicUsize::new(0));
    let build_count = Arc::clone(&builds);
    let worker = spawn_rebuild_worker(LiveReload::new("/reload"), move || {
        build_count.fetch_add(1, Ordering::Relaxed);
        started.send(()).map_err(|e| e.to_string())?;
        continue_work.recv().map_err(|e| e.to_string())?;
        if hung {
            hang();
        }
        fs::write(&completed, b"completed").map_err(|e| e.to_string())?;
        Ok(Vec::new())
    });
    let sender = worker.sender();
    sender.try_send(Vec::new())?;
    active.recv_timeout(Duration::from_secs(5))?;
    for _ in 0..8 {
        sender.try_send(Vec::new())?;
    }
    let server_result = actix_web::rt::System::new().block_on(async {
        let builder = HttpServer::new(|| {
            App::new().route(
                "/",
                web::get().to(|| async { HttpResponse::Ok().body("ready") }),
            )
        })
        .disable_signals()
        .workers(1)
        .bind(("127.0.0.1", 0))
        .map_err(Error::Http)?;
        let address = builder
            .addrs()
            .first()
            .copied()
            .ok_or_else(|| Error::Http(io::Error::other("missing HTTP bind address")))?;
        let server = builder.run();
        fs::write(output.join("address"), address.to_string()).map_err(Error::Http)?;
        fs::write(output.join("root.ready"), b"ready").map_err(Error::Http)?;
        control.serve(server).await
    });
    // Model the real CLI order: stop HTTP, discard the watcher, join the worker
    // on BOTH success and error. Keep a sender clone alive throughout the join.
    let joiner = thread::spawn(move || worker.shutdown());
    fs::write(output.join("joining"), b"HTTP returned")?;
    if !hung {
        wait_for(&output.join("release"))?;
    }
    release.send(())?;
    joiner.join().map_err(|_| "joiner panicked")??;
    fs::write(
        output.join("build-count"),
        builds.load(Ordering::Relaxed).to_string(),
    )?;
    drop(sender);
    match server_result {
        Ok(()) => Ok(JOINED_EXIT_CODE),
        Err(Error::Control(_)) => Ok(24),
        Err(error) => Err(error.into()),
    }
}
