// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;
use actix_web::{App, HttpResponse, HttpServer};
use std::fs;
use std::net::TcpStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const FIXTURE_MODE: &str = "_WEBUI_SHUTDOWN_TEST_MODE";
const FIXTURE_OUTPUT: &str = "_WEBUI_SHUTDOWN_TEST_OUTPUT";

#[test]
fn no_policy_is_direct_and_large_timeout_is_rejected() -> TestResult {
    assert!(matches!(prepare(None)?, Mode::Direct));
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
        App::new().default_service(actix_web::web::to(|| async { HttpResponse::Ok() }))
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
#[ignore = "subprocess fixture"]
fn fixture() {
    let result = fixture_main();
    std::process::exit(result.unwrap_or(91));
}

fn fixture_main() -> TestResult<i32> {
    let output = std::env::var_os(FIXTURE_OUTPUT).ok_or("missing fixture output")?;
    let output = Path::new(&output);
    match std::env::var(FIXTURE_MODE)?.as_str() {
        "writer" => {
            let path = output.join("writes");
            loop {
                fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)?
                    .write_all(b"x")?;
                thread::sleep(Duration::from_millis(10));
            }
        }
        mode => {
            let mut gate = [0_u8; 1];
            std::io::stdin().read_exact(&mut gate)?;
            if gate != *b"G" {
                return Ok(91);
            }
            if mode == "hung" {
                fixture_command("writer", output)?.spawn()?;
            }
            println!("fixture ready");
            fs::write(output.join("ready"), b"ready")?;
            let mut stop = [0_u8; 1];
            std::io::stdin().read_exact(&mut stop)?;
            if mode == "graceful" && stop == *b"S" {
                fs::write(output.join("complete"), b"done")?;
                return Ok(JOINED_EXIT_CODE);
            }
            loop {
                thread::park();
            }
        }
    }
}

struct Running {
    output: TempDir,
    stop: StopHandle,
    runner: Option<thread::JoinHandle<Result<i32, Error>>>,
}

impl Running {
    fn start(mode: &str, grace: Duration) -> TestResult<Self> {
        let root = std::env::current_dir()?
            .join("target")
            .join("shutdown-tests");
        fs::create_dir_all(&root)?;
        let output = tempfile::tempdir_in(root)?;
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

fn fixture_command(mode: &str, output: &Path) -> std::io::Result<Command> {
    let mut command = Command::new(std::env::current_exe()?);
    command.args([
        "--exact",
        "shutdown::tests::fixture",
        "--ignored",
        "--nocapture",
        "--test-threads=1",
    ]);
    command
        .env(FIXTURE_MODE, mode)
        .env(FIXTURE_OUTPUT, output)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    Ok(command)
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
