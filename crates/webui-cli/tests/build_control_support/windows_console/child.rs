// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use windows_sys::Win32::System::Threading::CREATE_NEW_CONSOLE;

use super::platform::{clear_inherited_ignore, Job, Process};

const DEADLINE: Duration = Duration::from_secs(25);

pub(super) struct ConsoleChild {
    child: Child,
    job: Job,
    lines: Receiver<String>,
    log: PathBuf,
}

impl ConsoleChild {
    pub(super) fn spawn(role: &str) -> Result<Self> {
        let job = Job::new()?;
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("test-results")
            .join("client-builder");
        fs::create_dir_all(&directory)?;
        let log = directory.join(format!("console-{}-{role}.log", std::process::id()));
        let output = File::create(&log)?;
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args(["--exact", "windows_console::console_child", "--nocapture"])
            .env("WEBUI_CONSOLE_TEST_ROLE", role)
            .env("WEBUI_CONSOLE_TEST_PARENT", std::process::id().to_string())
            .creation_flags(CREATE_NEW_CONSOLE)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(output.try_clone()?);
        clear_inherited_ignore()?;
        let child = command
            .spawn()
            .context("start separately isolated console")?;
        let (send, lines) = mpsc::channel();
        let mut owned = Self {
            child,
            job,
            lines,
            log,
        };
        // The helper cannot spawn anything until this assignment and the start
        // message. Closing our non-inherited job handle contains timeout/panic
        // cleanup too, including grandchildren, without PID/name-based killing.
        owned.job.assign(&owned.child)?;
        let stdout = owned.child.stdout.take().context("console stdout")?;
        thread::spawn(move || {
            let mut output = output;
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let _ = writeln!(output, "{line}");
                let _ = output.flush();
                let _ = send.send(line);
            }
        });
        owned.send("start")?;
        Ok(owned)
    }

    pub(super) fn send(&mut self, message: &str) -> Result<()> {
        let input = self.child.stdin.as_mut().context("console child stdin")?;
        writeln!(input, "{message}")?;
        input.flush()?;
        Ok(())
    }

    pub(super) fn receive(&self, prefix: &str) -> Result<String> {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let line = self
                .lines
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .with_context(|| format!("waiting for {prefix}; log: {}", self.log.display()))?;
            if let Some(value) = line.strip_prefix(prefix) {
                return Ok(value.to_owned());
            }
        }
    }

    pub(super) fn processes(&self) -> Result<Vec<Process>> {
        self.job.processes()
    }

    pub(super) fn wait(&mut self) -> Result<ExitStatus> {
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                bail!("console child did not exit; log: {}", self.log.display());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    pub(super) fn assert_empty(&self) -> Result<()> {
        // Console-host teardown and job exit accounting can trail process exit.
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let ids = self.job.ids()?;
            if ids.is_empty() {
                return Ok(());
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "owned processes survived graceful exit: {ids:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    pub(super) fn logs(&self) -> Result<String> {
        Ok(fs::read_to_string(&self.log)?)
    }
}

impl Drop for ConsoleChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // The job's RAII close kills any remaining descendants on every path.
    }
}
