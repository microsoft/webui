// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

const SCRIPT: &str = include_str!("client_worker.mjs");
const BUILTIN_SCRIPT: &str = concat!(
    include_str!("builtin_builder.mjs"),
    "\n",
    include_str!("client_worker.mjs")
);
const MAX_RECORD: usize = 64 * 1024;

// A build-hook failure can be retried; a broken worker must be restarted.
#[derive(Debug, thiserror::Error)]
pub(super) enum BuildError {
    // The hook rejected this build, but its context remains available.
    #[error("Client build failed: {0}\nhelp: Fix the build error and save an input to retry.")]
    Build(String),
    // The worker or its private transport is no longer usable.
    #[error("Client builder runtime failed: {0}\nhelp: Fix the builder and restart the server.")]
    Runtime(#[source] anyhow::Error),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
enum Record {
    Ready {
        #[serde(rename = "watchPaths")]
        watch_paths: Vec<PathBuf>,
    },
    Built {},
    Error {
        message: String,
    },
    Stopped {},
}

// One warm client-build context with a bounded, sequential private transport.
pub(super) struct ClientBuilder {
    child: Option<Child>,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    frame: Vec<u8>,
    watch_paths: Vec<PathBuf>,
    timeout: Duration,
    usable: bool,
    closed: bool,
}

impl ClientBuilder {
    // Spawn a custom builder without surrendering ownership during initialization.
    pub(super) fn spawn(
        module: &Path,
        app_dir: &Path,
        out_dir: &Path,
        timeout: Duration,
    ) -> Result<Self> {
        Self::launch(
            SCRIPT,
            &[module.as_os_str(), app_dir.as_os_str(), out_dir.as_os_str()],
            timeout,
        )
    }

    // Spawn the embedded esbuild factory; await readiness with initialize.
    pub(super) fn spawn_builtin(
        app_dir: &Path,
        out_dir: &Path,
        client_entry: &Path,
        timeout: Duration,
    ) -> Result<Self> {
        Self::launch(
            BUILTIN_SCRIPT,
            &[
                OsStr::new("--builtin"),
                app_dir.as_os_str(),
                out_dir.as_os_str(),
                client_entry.as_os_str(),
            ],
            timeout,
        )
    }

    fn launch(script: &str, arguments: &[&OsStr], timeout: Duration) -> Result<Self> {
        let mut command = Command::new("node");
        // Only the CLI owns console shutdown. A Windows console broadcasts Ctrl+C
        // to attached children too, killing Node before the stop/EOF disposal
        // handshake. This pipe-only worker (and esbuild) needs no console.
        #[cfg(windows)]
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
        let mut child = command
            .args(["--input-type=module", "--eval", script, "--"])
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .context(
                "Cannot start client builder; install Node.js on PATH, then restart the server",
            )?;
        let input = child
            .stdin
            .take()
            .context("Cannot open client builder input")?;
        let output = child
            .stdout
            .take()
            .context("Cannot open client builder output")?;
        Ok(Self {
            child: Some(child),
            input: Some(input),
            output: BufReader::new(output),
            frame: Vec::with_capacity(1024),
            watch_paths: Vec::new(),
            timeout,
            usable: false,
            closed: false,
        })
    }

    // Await readiness while retaining the worker for explicit cleanup on cancellation.
    pub(super) async fn initialize(&mut self) -> Result<()> {
        if self.closed {
            bail!("Client builder is closed; spawn a new worker before initializing");
        }
        if self.usable {
            return Ok(());
        }
        let ready = tokio::time::timeout(self.timeout, self.receive()).await;
        let result = match ready {
            Ok(Ok(Record::Ready { watch_paths }))
                if watch_paths.iter().all(|path| path.is_absolute()) =>
            {
                self.watch_paths = watch_paths;
                self.usable = true;
                Ok(())
            }
            Ok(Ok(_)) => Err(anyhow::anyhow!(
                "Expected client builder readiness with absolute watchPaths"
            )),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(operation_timeout("initialization")),
        };
        if let Err(error) = result {
            let cleanup = self.disconnect().await;
            return Err(cleanup_context(error, cleanup).context(
                "Cannot initialize client builder; check its default factory and stderr diagnostics, then restart",
            ));
        }
        Ok(())
    }

    // Extra watch roots captured once from the initialized factory.
    pub(super) fn watch_paths(&self) -> &[PathBuf] {
        &self.watch_paths
    }

    // Await all client work before allowing the native build to start.
    pub(super) async fn rebuild(&mut self) -> Result<(), BuildError> {
        if !self.usable || self.closed {
            return Err(BuildError::Runtime(anyhow::anyhow!(
                "Client builder is closed or an earlier operation was interrupted"
            )));
        }
        // Cancellation must not let a later request consume an earlier reply.
        self.usable = false;
        let reply =
            tokio::time::timeout(self.timeout, self.exchange(b"{\"type\":\"build\"}\n")).await;
        let result = match reply {
            Ok(Ok(Record::Built {})) => Ok(()),
            Ok(Ok(Record::Error { message })) if !message.is_empty() => {
                Err(BuildError::Build(message))
            }
            Ok(Ok(_)) => Err(BuildError::Runtime(anyhow::anyhow!(
                "Unexpected client builder reply; use console logging instead of raw file-descriptor writes"
            ))),
            Ok(Err(error)) => Err(BuildError::Runtime(error)),
            Err(_) => Err(BuildError::Runtime(operation_timeout("rebuild"))),
        };
        if let Err(BuildError::Runtime(error)) = result {
            let cleanup = self.disconnect().await;
            return Err(BuildError::Runtime(cleanup_context(error, cleanup)));
        }
        self.usable = true;
        result
    }

    // Wait for idle worker exit without consuming output; cancellation is safe.
    pub(super) async fn exited(&mut self) -> std::io::Result<ExitStatus> {
        self.child
            .as_mut()
            .ok_or_else(|| std::io::Error::other("Client builder process is unavailable"))?
            .wait()
            .await
    }

    // Dispose once, then reap the worker; force cleanup after failure.
    pub(super) async fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        if !self.usable {
            let status = self.disconnect().await?;
            if !status.success() {
                bail!("Interrupted client builder cleanup failed ({status}); fix dispose() or increase --client-build-timeout-ms");
            }
            return Ok(());
        }
        self.usable = false;
        let result = tokio::time::timeout(self.timeout, self.stop()).await;
        match result {
            Ok(Ok(())) => {
                self.closed = true;
                self.input.take();
                Ok(())
            }
            failure => {
                self.terminate().await?;
                match failure {
                    Ok(Err(error)) => Err(error.context(
                        "Client builder disposal failed; fix dispose() and restart the server",
                    )),
                    _ => Err(operation_timeout("disposal")),
                }
            }
        }
    }

    async fn stop(&mut self) -> Result<()> {
        if !matches!(
            self.exchange(b"{\"type\":\"stop\"}\n").await?,
            Record::Stopped {}
        ) {
            bail!("Expected client builder disposal acknowledgement");
        }
        self.input.take();
        let status = self
            .process()?
            .wait()
            .await
            .context("Cannot reap client builder")?;
        if !status.success() {
            bail!("Client builder exited with {status}; check its stderr diagnostics");
        }
        // A stopped acknowledgement cannot hide trailing garbage or another reply.
        if !self.output.fill_buf().await?.is_empty() {
            bail!("Unexpected output after client builder disposal");
        }
        Ok(())
    }

    async fn exchange(&mut self, command: &[u8]) -> Result<Record> {
        if !self.frame.is_empty() || !self.output.buffer().is_empty() {
            bail!("Unsolicited client builder output; use console logging instead of raw file-descriptor writes");
        }
        self.check_running()?;
        self.input
            .as_mut()
            .context("Client builder input is closed")?
            .write_all(command)
            .await
            .context("Cannot send client builder request; restart the server")?;
        self.receive().await
    }

    async fn receive(&mut self) -> Result<Record> {
        let record = read_record(&mut self.output, &mut self.frame).await?;
        if !self.output.buffer().is_empty() {
            bail!("Unexpected extra client builder output; use console logging instead of raw file-descriptor writes");
        }
        if !matches!(record, Record::Stopped {}) {
            self.check_running()?;
        }
        Ok(record)
    }

    fn check_running(&mut self) -> Result<()> {
        if let Some(status) = self
            .process()?
            .try_wait()
            .context("Cannot inspect client builder")?
        {
            bail!("Client builder exited with {status}; check stderr and restart the server");
        }
        Ok(())
    }

    async fn terminate(&mut self) -> Result<()> {
        self.usable = false;
        self.input.take();
        self.process()?
            .kill()
            .await
            .context("Cannot terminate and reap client builder")?;
        self.closed = true;
        Ok(())
    }

    fn process(&mut self) -> Result<&mut Child> {
        self.child
            .as_mut()
            .context("Client builder process is unavailable")
    }

    async fn disconnect(&mut self) -> Result<ExitStatus> {
        self.usable = false;
        self.input.take();
        let child = self
            .child
            .as_mut()
            .context("Client builder process is unavailable")?;
        match tokio::time::timeout(self.timeout, drain_and_wait(child, &mut self.output)).await {
            Ok(Ok(status)) => {
                self.closed = true;
                Ok(status)
            }
            failure => {
                self.terminate().await?;
                match failure {
                    Ok(Err(error)) => Err(error.into()),
                    _ => Err(operation_timeout("EOF cleanup")),
                }
            }
        }
    }
}

async fn drain_and_wait(
    child: &mut Child,
    output: &mut BufReader<ChildStdout>,
) -> std::io::Result<ExitStatus> {
    let mut buffer = [0_u8; 1024];
    while output.read(&mut buffer).await? != 0 {}
    child.wait().await
}

impl Drop for ClientBuilder {
    fn drop(&mut self) {
        self.input.take();
        let Some(mut child) = self.child.take() else {
            return;
        };
        if self.closed {
            return;
        }
        // If the runtime is unavailable or stops this task, kill_on_drop remains
        // the fallback. An active runtime gives hooks a bounded EOF cleanup.
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let timeout = self.timeout;
        runtime.spawn(async move {
            if !matches!(tokio::time::timeout(timeout, child.wait()).await, Ok(Ok(_))) {
                let _ = child.kill().await;
            }
        });
    }
}

async fn read_record(output: &mut BufReader<ChildStdout>, frame: &mut Vec<u8>) -> Result<Record> {
    loop {
        let available = output
            .fill_buf()
            .await
            .context("Cannot read client builder output")?;
        if available.is_empty() {
            bail!("Client builder output closed; check stderr diagnostics and restart the server");
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        if consumed > MAX_RECORD - frame.len() {
            bail!("Client builder output exceeded 64 KiB; use console logging for diagnostics");
        }
        frame.extend_from_slice(&available[..consumed]);
        output.consume(consumed);
        if newline.is_some() {
            let record = serde_json::from_slice(frame).context(
                "Malformed client builder output; use console logging instead of raw file-descriptor writes",
            );
            frame.clear();
            return record;
        }
    }
}

#[cold]
fn operation_timeout(operation: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "Client builder {operation} timed out; fix hung hooks or increase --client-build-timeout-ms, then restart"
    )
}

#[cold]
fn cleanup_context(error: anyhow::Error, cleanup: Result<ExitStatus>) -> anyhow::Error {
    match cleanup {
        Ok(_) => error,
        Err(cleanup) => error.context(format!("Client builder cleanup also failed: {cleanup:#}")),
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests;
