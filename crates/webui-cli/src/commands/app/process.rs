// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Child;

use crate::commands::native::control::Stop;
use crate::commands::native::NativeError;

fn spawn(mut command: Command, capture: bool) -> Result<Child> {
    let program = command.get_program().to_owned();
    command.stdin(Stdio::null()).stderr(Stdio::inherit());
    if capture {
        command.stdout(Stdio::piped());
    } else {
        command.stdout(Stdio::from(std::io::stderr()));
    }
    tokio::process::Command::from(command)
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| NativeError::Host(format!("could not launch {program:?}: {error}")).into())
}

pub(super) async fn run(command: Command, stop: &mut Stop) -> Result<()> {
    let program = command.get_program().to_owned();
    let mut child = spawn(command, false)?;
    tokio::select! {
        reason = stop.requested() => {
            stop.abort_build(&mut child).await?;
            reason?;
            Err(NativeError::Cancelled.into())
        }
        status = child.wait() => {
            let status = status.map_err(|error| NativeError::Host(error.to_string()))?;
            if status.success() { Ok(()) } else {
                Err(NativeError::Host(format!("{program:?} exited with {status}")).into())
            }
        }
    }
}

pub(super) async fn cargo(command: Command, target: &str, stop: &mut Stop) -> Result<PathBuf> {
    let mut child = spawn(command, true)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| NativeError::Host("Cargo stdout was not captured".into()))?;
    let mut reader = BufReader::new(stdout);
    let mut buffer = Vec::with_capacity(8192);
    let mut executable = None;
    loop {
        buffer.clear();
        let count = tokio::select! {
            reason = stop.requested() => {
                stop.abort_build(&mut child).await?;
                reason?;
                return Err(NativeError::Cancelled.into());
            }
            count = async {
                let mut bounded = (&mut reader).take(8 * 1024 * 1024 + 1);
                bounded.read_until(b'\n', &mut buffer).await
            } => count.map_err(|error| NativeError::Host(error.to_string()))?,
        };
        if count == 0 {
            break;
        }
        let record = if count > 8 * 1024 * 1024 {
            Err(NativeError::Host("Cargo JSON record exceeded 8 MiB".into()).into())
        } else {
            serde_json::from_slice::<serde_json::Value>(&buffer).map_err(|error| {
                NativeError::Host(format!("invalid Cargo artifact output: {error}")).into()
            })
        };
        let record: serde_json::Value = match record {
            Ok(record) => record,
            Err(error) => {
                stop.abort_build(&mut child).await?;
                return Err(error);
            }
        };
        if record["reason"] == "compiler-artifact"
            && record["target"]["name"] == target
            && record["target"]["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "bin"))
        {
            if let Some(path) = record["executable"].as_str() {
                executable = Some(PathBuf::from(path));
            }
        }
    }
    let status = tokio::select! {
        reason = stop.requested() => {
            stop.abort_build(&mut child).await?;
            reason?;
            return Err(NativeError::Cancelled.into());
        }
        status = child.wait() => status.map_err(|error| NativeError::Host(error.to_string()))?,
    };
    if !status.success() {
        return Err(NativeError::Host(format!("Cargo exited with {status}")).into());
    }
    executable.ok_or_else(|| {
        NativeError::Host(format!("Cargo did not report the {target} executable")).into()
    })
}

pub(super) struct Server {
    pub child: Child,
    control: Option<tokio::process::ChildStdin>,
}

impl Server {
    pub(super) fn start(mut command: Command) -> Result<Self> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .env("_WEBUI_APP_CONTROL", "stdin-v1");
        let mut child = tokio::process::Command::from(command)
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| NativeError::Host(error.to_string()))?;
        // Child::wait closes its stdin; supervision must retain the control pipe.
        let control = child.stdin.take();
        Ok(Self { child, control })
    }

    pub(super) async fn stop(mut self) -> Result<()> {
        if let Some(status) = self.child.try_wait()? {
            return if status.success() {
                Ok(())
            } else {
                Err(
                    NativeError::Cleanup(format!("native host exited before draining: {status}"))
                        .into(),
                )
            };
        }
        let mut input = self.control.take().ok_or_else(|| {
            NativeError::Cleanup("native host has no shutdown control pipe".into())
        })?;
        if let Err(error) = input.write_all(b"S").await {
            if self.child.try_wait()?.is_none() {
                return Err(NativeError::Cleanup(format!(
                    "could not request native host shutdown: {error}"
                ))
                .into());
            }
        }
        drop(input);
        let status = tokio::time::timeout(Duration::from_secs(11), self.child.wait())
            .await
            .map_err(|_| {
                NativeError::Cleanup("native host exceeded its ten-second drain allowance".into())
            })??;
        if !status.success() {
            return Err(
                NativeError::Cleanup(format!("native host shutdown returned {status}")).into(),
            );
        }
        Ok(())
    }
}
