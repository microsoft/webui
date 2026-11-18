// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::notification::StopHandle;
use super::*;
use actix_web::{App, HttpResponse, HttpServer};
use fixtures::{fixture_command, fixture_main, fixture_output};
use std::fs;
use std::io::Write;
use std::net::TcpStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

mod fixtures;
mod stdin;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const FIXTURE_MODE: &str = "_WEBUI_SHUTDOWN_TEST_MODE";
const FIXTURE_OUTPUT: &str = "_WEBUI_SHUTDOWN_TEST_OUTPUT";

#[test]
fn no_policy_is_direct_and_large_timeout_is_rejected() -> TestResult {
    assert!(matches!(prepare(None)?, Mode::Direct));
    assert!(matches!(prepare_with_stdin(None, true)?, Mode::Direct));
    assert!(matches!(
        prepare(NonZeroU64::new(u64::MAX)),
        Err(Error::InvalidPolicy)
    ));
    Ok(())
}

#[test]
fn spawn_failure_does_not_fall_back_to_direct_execution() {
    let (_stop, requests) = stop_channel();
    assert!(matches!(
        supervise(
            Command::new("webui-no-such-shutdown-fixture"),
            Duration::from_secs(1),
            requests,
        ),
        Err(Error::Startup(_))
    ));
}

#[actix_web::test]
async fn control_stops_the_http_server() -> TestResult {
    let builder = HttpServer::new(|| {
        App::new().default_service(actix_web::web::to(|| async { HttpResponse::Ok().finish() }))
    })
    .disable_signals()
    .workers(1)
    .bind(("127.0.0.1", 0))?;
    let address = builder
        .addrs()
        .first()
        .copied()
        .ok_or("missing test server address")?;
    let server = builder.run();
    let (sender, receiver) = watch::channel(ControlState::Running);
    let task = actix_web::rt::spawn(Control(receiver).serve(server));
    wait_until(|| Ok(TcpStream::connect(address).is_ok()))?;
    sender.send(ControlState::Stop)?;
    task.await??;
    wait_until(|| Ok(TcpStream::connect(address).is_err()))?;
    Ok(())
}

#[test]
fn graceful_stop_waits_for_child_cleanup() -> TestResult {
    let mut running = Running::start("graceful", Duration::from_secs(2))?;
    assert!(running.stop.request());
    assert_eq!(running.finish()?, 0);
    assert_eq!(fs::read(running.output.path().join("complete"))?, b"done");
    Ok(())
}

#[test]
fn deadline_kills_the_owned_process_group() -> TestResult {
    let mut running = Running::start("hung", Duration::from_millis(150))?;
    let started = Instant::now();
    wait_for(&running.output.path().join("writes"))?;
    assert!(running.stop.request());
    assert!(matches!(
        running.finish(),
        Err(Error::Forced(ForcedReason::Deadline))
    ));
    assert!(started.elapsed() >= Duration::from_millis(150));
    assert!(started.elapsed() < Duration::from_secs(3));
    let before = fs::metadata(running.output.path().join("writes"))?.len();
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        fs::metadata(running.output.path().join("writes"))?.len(),
        before
    );
    Ok(())
}

#[test]
fn second_request_forces_immediate_termination() -> TestResult {
    let mut running = Running::start("hung", Duration::from_secs(30))?;
    assert!(running.stop.request());
    let started = Instant::now();
    assert!(running.stop.request());
    assert!(matches!(
        running.finish(),
        Err(Error::Forced(ForcedReason::RepeatedRequest))
    ));
    assert!(started.elapsed() < Duration::from_secs(3));
    Ok(())
}

#[test]
fn coalesced_notifications_preserve_second_stop_and_source_loss() -> TestResult {
    let (stop, requests) = stop_channel();
    assert!(requests.notification.wake());
    // The single notification slot is already full.
    for _ in 0..128 {
        assert!(stop.request());
    }
    assert_eq!(requests.count(), 2);
    assert!(requests.wait(Some(Instant::now() + CONFIRMATION))?);
    drop(stop);
    assert!(requests.wait(Some(Instant::now() + CONFIRMATION))?);
    assert!(requests.disconnected());
    assert_eq!(requests.count(), 2);
    Ok(())
}

#[test]
fn exit_waiter_notifies_without_stop_and_retains_owned_group() -> TestResult {
    let output = fixture_output()?;
    let (_stop, requests) = stop_channel();
    let mut command = fixture_command("exit", output.path())?;
    let mut scope = platform::Scope::new()?;
    scope.configure(&mut command);
    let mut child = command.stdin(Stdio::piped()).spawn()?;
    scope.attach(&child)?;
    let mut control = child.stdin.take().ok_or("missing fixture control")?;
    let mut waiter = super::task::Task::spawn(
        "shutdown-test-exit",
        requests.notification.clone(),
        move || {
            let result = platform::wait_for_exit(&mut child);
            (child, result)
        },
    )?;
    control.write_all(b"G")?;
    // This asserts receipt of a real completion notification, not an idle
    // polling constant or an elapsed-time performance threshold.
    assert!(requests.wait(Some(Instant::now() + Duration::from_secs(10)))?);
    let (mut child, result) = waiter.take_result()?.ok_or("missing exit notification")?;
    result?;
    assert_eq!(requests.count(), 0);
    scope.terminate(Instant::now() + CONFIRMATION)?;
    assert_eq!(child.try_wait()?.and_then(|status| status.code()), Some(37));
    waiter.join_until(Instant::now() + CONFIRMATION)?;
    Ok(())
}

#[test]
fn startup_gate_prevents_descendants_before_release() -> TestResult {
    let output = fixture_output()?;
    let (_stop, requests) = stop_channel();
    let mut worker = Worker::spawn(
        fixture_command("hung", output.path())?,
        requests.notification.clone(),
    )?;
    wait_for(&output.path().join("at-gate"))?;
    assert!(!output.path().join("ready").exists());
    assert!(!output.path().join("writes").exists());
    worker.finish()?;
    assert!(!output.path().join("writes").exists());
    // Teardown cannot signal a group again after its leader has been reaped.
    assert!(matches!(
        worker.finish(),
        Err(Error::Control(error)) if error.kind() == io::ErrorKind::InvalidInput
    ));
    Ok(())
}

#[test]
fn normal_exit_notifies_and_kills_remaining_grandchildren() -> TestResult {
    let mut running = Running::start("exit-with-writer", Duration::from_secs(30))?;
    assert_eq!(running.finish()?, 37);
    let before = fs::metadata(running.output.path().join("writes"))?.len();
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        fs::metadata(running.output.path().join("writes"))?.len(),
        before
    );
    Ok(())
}

#[test]
fn stop_source_loss_still_joins_gracefully() -> TestResult {
    let mut running = Running::start("graceful", Duration::from_secs(2))?;
    let (replacement, _unused) = stop_channel();
    drop(std::mem::replace(&mut running.stop, replacement));
    assert_eq!(running.finish()?, 0);
    assert_eq!(fs::read(running.output.path().join("complete"))?, b"done");
    Ok(())
}

#[actix_web::test]
async fn native_control_reports_stop_and_control_loss() -> TestResult {
    let (sender, receiver) = watch::channel(ControlState::Running);
    let mut control = Control(receiver);
    sender.send(ControlState::Stop)?;
    control.shutdown_requested().await?;
    // Stop remains visible even if requested before the caller starts waiting.
    control.shutdown_requested().await?;
    let (sender, receiver) = watch::channel(ControlState::Running);
    let mut control = Control(receiver);
    drop(sender);
    assert!(matches!(
        control.shutdown_requested().await,
        Err(Error::Control(error)) if error.kind() == io::ErrorKind::UnexpectedEof
    ));
    Ok(())
}

#[test]
fn normal_consuming_caller_receives_all_final_output() -> TestResult {
    let output = fixture_output()?;
    let result = fixture_command("supervisor-verbose", output.path())?
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    assert!(result.status.success());
    assert_eq!(
        result.stdout.iter().filter(|&&byte| byte == b'~').count(),
        1024 * 1024
    );
    assert!(result
        .stderr
        .windows(b"final diagnostic".len())
        .any(|window| window == b"final diagnostic"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn blocked_relay_reports_failure_within_the_confirmation_bound() -> TestResult {
    let output = fixture_output()?;
    let mut child = fixture_command("supervisor-blocked", output.path())?
        .stdout(Stdio::piped())
        .spawn()?;
    wait_for(&output.path().join("ready"))?;
    let started = Instant::now();
    fs::write(output.path().join("request-stop"), b"stop")?;
    wait_until(|| Ok(child.try_wait()?.is_some()))?;
    assert!(
        child.wait()?.success(),
        "{}",
        fs::read_to_string(output.path().join("result"))?
    );
    assert!(started.elapsed() < CONFIRMATION + Duration::from_secs(1));
    assert!(fs::read_to_string(output.path().join("result"))?.contains("did not finish"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn broken_relay_wakes_idle_supervisor_and_terminates_worker() -> TestResult {
    let output = fixture_output()?;
    let mut child = fixture_command("supervisor-broken", output.path())?
        .stdout(Stdio::piped())
        .spawn()?;
    wait_for(&output.path().join("ready"))?;
    drop(child.stdout.take());
    wait_until(|| Ok(child.try_wait()?.is_some()))?;
    assert!(
        child.wait()?.success(),
        "{}",
        fs::read_to_string(output.path().join("result"))?
    );
    let result = fs::read_to_string(output.path().join("result"))?;
    assert!(result.contains("BrokenPipe"), "{result}");
    Ok(())
}

#[test]
#[ignore = "subprocess fixture"]
fn fixture() {
    let result = fixture_main();
    std::process::exit(result.unwrap_or(91));
}

struct Running {
    output: TempDir,
    stop: StopHandle,
    runner: Option<thread::JoinHandle<Result<i32, Error>>>,
}

impl Running {
    fn start(mode: &str, grace: Duration) -> TestResult<Self> {
        let output = fixture_output()?;
        let command = fixture_command(mode, output.path())?;
        let (stop, requests) = stop_channel();
        let runner = thread::spawn(move || supervise(command, grace, requests));
        let running = Self {
            output,
            stop,
            runner: Some(runner),
        };
        wait_for(&running.output.path().join("ready"))?;
        Ok(running)
    }

    fn finish(&mut self) -> Result<i32, Error> {
        self.runner
            .take()
            .ok_or_else(|| Error::Control(io::Error::other("supervisor already joined")))?
            .join()
            .map_err(|_| Error::Control(io::Error::other("supervisor thread panicked")))?
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(runner) = self.runner.take() {
            let _ = self.stop.request();
            let _ = self.stop.request();
            let _ = runner.join();
        }
    }
}

fn wait_for(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(
        path.parent()
            .ok_or_else(|| io::Error::other("fixture path has no parent"))?,
    )?;
    wait_until(|| Ok(path.exists()))
}

fn wait_until(mut ready: impl FnMut() -> std::io::Result<bool>) -> std::io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready()? {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "fixture did not become ready",
            ));
        }
        thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}
