// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::util::build_command;
use std::io::{Read, Seek, SeekFrom};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

pub(super) struct Captured {
    pub stdout: String,
    pub stderr: String,
    pub status: ExitStatus,
    pub pid: u32,
    pub timed_out: bool,
}

pub(super) fn command(program: &str, args: &[&str]) -> Command {
    build_command(program, args)
}

pub(super) fn run(command: &mut Command, label: &str, timeout: Duration) -> Result<(), String> {
    println!("+ {label}");
    let mut child = command
        .spawn()
        .map_err(|error| format!("{label}: {error}"))?;
    let (status, timed_out) = wait(&mut child, timeout)?;
    if timed_out || !status.success() {
        return Err(format!("{label}: timeout={timed_out} exit={status}"));
    }
    Ok(())
}

pub(super) fn capture(
    command: &mut Command,
    label: &str,
    timeout: Duration,
) -> Result<Captured, String> {
    println!("+ {label}");
    let mut stdout = tempfile::tempfile().map_err(|error| format!("{label}: {error}"))?;
    let mut stderr = tempfile::tempfile().map_err(|error| format!("{label}: {error}"))?;
    command
        .stdout(Stdio::from(
            stdout
                .try_clone()
                .map_err(|error| format!("{label}: {error}"))?,
        ))
        .stderr(Stdio::from(
            stderr
                .try_clone()
                .map_err(|error| format!("{label}: {error}"))?,
        ));
    let mut child = command
        .spawn()
        .map_err(|error| format!("{label}: {error}"))?;
    let pid = child.id();
    let (status, timed_out) = wait(&mut child, timeout)?;
    stdout
        .seek(SeekFrom::Start(0))
        .map_err(|error| format!("{label}: {error}"))?;
    stderr
        .seek(SeekFrom::Start(0))
        .map_err(|error| format!("{label}: {error}"))?;
    let mut stdout_text = String::new();
    let mut stderr_text = String::new();
    stdout
        .read_to_string(&mut stdout_text)
        .map_err(|error| format!("{label} stdout: {error}"))?;
    stderr
        .read_to_string(&mut stderr_text)
        .map_err(|error| format!("{label} stderr: {error}"))?;
    print!("{stdout_text}");
    eprint!("{stderr_text}");
    Ok(Captured {
        stdout: stdout_text,
        stderr: stderr_text,
        status,
        pid,
        timed_out,
    })
}

fn wait(child: &mut Child, timeout: Duration) -> Result<(ExitStatus, bool), String> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or("process timeout exceeds the supported duration")?;
    loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            return Ok((status, false));
        }
        if Instant::now() >= deadline {
            if let Err(error) = child.kill() {
                if child
                    .try_wait()
                    .map_err(|error| error.to_string())?
                    .is_none()
                {
                    return Err(format!("failed to stop timed-out process: {error}"));
                }
            }
            let status = child.wait().map_err(|error| error.to_string())?;
            return Ok((status, true));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
