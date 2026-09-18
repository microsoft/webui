// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Bounded process and HTTP helpers for native dev-server integration tests.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::NamedTempFile;

const TEST_TIMEOUT: Duration = Duration::from_secs(15);

/// A directly owned test server with captured diagnostics and cleanup on failure.
///
/// Use only for fixtures without build subprocesses. On Unix the guard first
/// requests server/supervisor shutdown; process-tree tests own their containment.
pub struct TestServer {
    child: Child,
    diagnostics: NamedTempFile,
}

impl TestServer {
    /// Spawn the actual CLI in its private controlled-child mode.
    ///
    /// This deliberately exercises the production startup protocol rather than
    /// exposing a test-only gate in either binary.
    pub fn spawn_gated(command: &mut Command) -> io::Result<Self> {
        command.env("_WEBUI_DEV_SERVER_CHILD", "joined-v1");
        Self::spawn(command)
    }

    /// Spawn a server with private piped stdin and file-backed diagnostics.
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        let diagnostics = NamedTempFile::new()?;
        let output = diagnostics.reopen()?;
        let child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::from(output.try_clone()?))
            .stderr(Stdio::from(output))
            .spawn()?;
        Ok(Self { child, diagnostics })
    }

    /// Send one startup/shutdown protocol command.
    pub fn send(&mut self, command: u8) -> io::Result<()> {
        self.child
            .stdin
            .as_mut()
            .ok_or_else(|| io::Error::other("server stdin is closed"))?
            .write_all(&[command])
    }

    /// Close the controlling endpoint to model a lost supervisor.
    pub fn close_control(&mut self) {
        drop(self.child.stdin.take());
    }

    /// Send an ordinary Unix stop signal to this exact server PID.
    #[cfg(unix)]
    pub fn signal(&self, signal: &str) -> io::Result<()> {
        let status = Command::new("kill")
            .args(["-s", signal, &self.child.id().to_string()])
            .status()?;
        if !status.success() {
            return Err(io::Error::other(format!(
                "kill -s {signal} failed: {status}"
            )));
        }
        Ok(())
    }

    /// Confirm that a gated server remains alive without starting work.
    pub fn ensure_running(&mut self) -> io::Result<()> {
        if let Some(status) = self.child.try_wait()? {
            return Err(self.failure(&format!("server exited early: {status}")));
        }
        Ok(())
    }

    /// Wait for an HTTP response containing the expected rendered content.
    pub fn wait_for_content(&mut self, address: SocketAddr, content: &str) -> io::Result<String> {
        let deadline = Instant::now() + TEST_TIMEOUT;
        loop {
            self.ensure_running()?;
            match http_get(address) {
                Ok(response) if response.contains(content) => return Ok(response),
                _ if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
                result => return Err(self.failure(&format!("HTTP content not ready: {result:?}"))),
            }
        }
    }

    /// Wait for a process exit, failing instead of hanging the test indefinitely.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        let deadline = Instant::now() + TEST_TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                return Err(self.failure("server did not exit"));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Read the child's combined stdout/stderr diagnostics.
    pub fn diagnostics(&self) -> io::Result<String> {
        std::fs::read_to_string(self.diagnostics.path())
    }

    fn failure(&self, message: &str) -> io::Error {
        match self.diagnostics() {
            Ok(output) => io::Error::other(format!("{message}\n{output}")),
            Err(error) => io::Error::other(format!("{message}; cannot read diagnostics: {error}")),
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        #[cfg(unix)]
        if matches!(self.child.try_wait(), Ok(None)) {
            // A supervised fixture owns a separate group. Give its real stop
            // path a chance to clean that scope before killing the supervisor.
            let _ = self.signal("TERM");
            let _ = self.signal("TERM");
            let _ = self.wait();
        }
        match self.child.try_wait() {
            Ok(Some(_)) => {}
            _ => {
                if let Err(error) = self.child.kill() {
                    eprintln!("Cannot terminate test server {}: {error}", self.child.id());
                }
                if let Err(error) = self.child.wait() {
                    eprintln!("Cannot reap test server {}: {error}", self.child.id());
                }
            }
        }
    }
}

fn http_get(address: SocketAddr) -> io::Result<String> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(200))?;
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    stream.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
    let mut response = String::with_capacity(4096);
    stream.read_to_string(&mut response)?;
    Ok(response)
}
