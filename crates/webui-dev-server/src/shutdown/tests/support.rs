// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::super::{supervisor, Error};
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tempfile::TempDir;

pub(super) type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
pub(super) const FIXTURE_MODE: &str = "_WEBUI_SHUTDOWN_TEST_MODE";
pub(super) const FIXTURE_OUTPUT: &str = "_WEBUI_SHUTDOWN_TEST_OUTPUT";

pub(in crate::shutdown) fn output() -> io::Result<TempDir> {
    // Keep test artifacts inside this checkout, never in the system temp area.
    let root = std::env::current_dir()?
        .join("target")
        .join("shutdown-tests");
    fs::create_dir_all(&root)?;
    tempfile::Builder::new().prefix("case-").tempdir_in(root)
}

pub(in crate::shutdown) fn command(mode: &str, output: &Path) -> io::Result<Command> {
    let mut command = Command::new(std::env::current_exe()?);
    command.args([
        "--exact",
        "shutdown::tests::fixtures::process_entry",
        "--ignored",
        "--nocapture",
        "--test-threads=1",
    ]);
    command.env(FIXTURE_MODE, mode).env(FIXTURE_OUTPUT, output);
    command.stdout(Stdio::null()).stderr(Stdio::null());
    Ok(command)
}

pub(super) struct Running {
    output: TempDir,
    stop: Option<supervisor::StopHandle>,
    runner: Option<JoinHandle<Result<i32, Error>>>,
}

impl Running {
    pub(super) fn start(mode: &str, grace: Duration) -> TestResult<Self> {
        let output = output()?;
        let command = command(mode, output.path())?;
        let (stop, requests) = supervisor::stop_channel();
        let runner = thread::spawn(move || supervisor::supervise(command, grace, requests));
        let running = Self {
            output,
            stop: Some(stop),
            runner: Some(runner),
        };
        wait_for(&running.path().join("root.ready"))?;
        Ok(running)
    }

    pub(super) fn path(&self) -> &Path {
        self.output.path()
    }

    pub(super) fn stop(&self) -> bool {
        self.stop
            .as_ref()
            .is_some_and(supervisor::StopHandle::request)
    }

    pub(super) fn disconnect_requests(&mut self) {
        self.stop.take();
    }

    pub(super) fn finish(&mut self) -> Result<i32, Error> {
        let runner = self
            .runner
            .take()
            .ok_or_else(|| Error::Supervision(io::Error::other("supervisor already joined")))?;
        runner
            .join()
            .map_err(|_| Error::Supervision(io::Error::other("supervisor thread panicked")))?
    }

    pub(super) fn assert_writes_stopped(&self) -> TestResult {
        let before = fs::read(self.path().join("writes"))?;
        thread::sleep(Duration::from_millis(100));
        assert_eq!(fs::read(self.path().join("writes"))?, before);
        Ok(())
    }

    pub(super) fn assert_http_ready(&self) -> TestResult {
        let address = fs::read_to_string(self.path().join("address"))?;
        let mut stream = std::net::TcpStream::connect(address)?;
        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
        stream.set_write_timeout(Some(Duration::from_secs(3)))?;
        stream.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.ends_with("ready"));
        Ok(())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(runner) = self.runner.take() {
            self.stop();
            self.stop();
            // Always join, even during assertions/unwinding. No test may leave
            // a detached supervisor or in-process rebuild behind.
            let _ = runner.join();
        }
    }
}

pub(super) fn wait_for(path: &Path) -> io::Result<()> {
    wait_until(|| Ok(path.exists()))
        .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", path.display())))
}

pub(super) fn wait_until(mut ready: impl FnMut() -> io::Result<bool>) -> io::Result<()> {
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
