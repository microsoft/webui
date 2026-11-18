// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{
    prepare_with_stdin, stop_channel, supervise, wait_for, Error, Mode, NonZeroU64, TestResult,
    FIXTURE_MODE, FIXTURE_OUTPUT, JOINED_EXIT_CODE,
};
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

pub(super) fn fixture_main() -> TestResult<i32> {
    let output = std::env::var_os(FIXTURE_OUTPUT).ok_or("missing fixture output")?;
    let output = Path::new(&output);
    match std::env::var(FIXTURE_MODE)?.as_str() {
        mode @ ("stdin-enabled" | "stdin-disabled" | "stdin-complete" | "stdin-hung") => {
            stdin_supervisor(mode, output)
        }
        "supervisor-idle" => {
            let (stop, requests) = stop_channel();
            ctrlc::set_handler(move || {
                stop.request();
            })?;
            Ok(supervise(
                fixture_command("graceful", output)?,
                Duration::from_secs(30),
                requests,
            )?)
        }
        "supervisor-verbose" => {
            let (_stop, requests) = stop_channel();
            let mut command = fixture_command("verbose", output)?;
            command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
            Ok(
                if supervise(command, Duration::from_secs(30), requests)? == 37 {
                    0
                } else {
                    92
                },
            )
        }
        "supervisor-blocked" | "supervisor-broken" => relay_supervisor(output),
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
        mode => worker(mode, output),
    }
}

fn stdin_supervisor(mode: &str, output: &Path) -> TestResult<i32> {
    match prepare_with_stdin(NonZeroU64::new(1), mode != "stdin-disabled") {
        Ok(Mode::Child(mut control)) => {
            fs::write(output.join("ready"), b"ready")?;
            if mode == "stdin-complete" {
                return Ok(37);
            }
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()?;
            if mode == "stdin-disabled" {
                return runtime.block_on(async {
                    match tokio::time::timeout(
                        Duration::from_millis(150),
                        control.shutdown_requested(),
                    )
                    .await
                    {
                        Err(_) => Ok(JOINED_EXIT_CODE),
                        Ok(result) => {
                            result?;
                            fs::write(output.join("complete"), b"unexpected stop")?;
                            Ok(94)
                        }
                    }
                });
            }
            runtime.block_on(control.shutdown_requested())?;
            if mode == "stdin-hung" {
                fs::write(output.join("stopping"), b"stopping")?;
                loop {
                    thread::park();
                }
            }
            fs::write(output.join("complete"), b"done")?;
            Ok(JOINED_EXIT_CODE)
        }
        Ok(Mode::Supervisor(code)) => {
            fs::write(output.join("result"), format!("Ok({code})"))?;
            Ok(code)
        }
        Ok(Mode::Direct) => Err("fixture unexpectedly skipped supervision".into()),
        Err(error) => {
            fs::write(output.join("result"), format!("{error:?}"))?;
            Ok(if matches!(error, Error::Forced(_)) {
                1
            } else {
                74
            })
        }
    }
}

fn relay_supervisor(output: &Path) -> TestResult<i32> {
    let blocked = std::env::var(FIXTURE_MODE)? == "supervisor-blocked";
    let (stop, requests) = stop_channel();
    let stopper = if blocked {
        let path = output.join("request-stop");
        Some(thread::spawn(move || {
            wait_for(&path)?;
            stop.request();
            Ok::<_, io::Error>(())
        }))
    } else {
        None
    };
    let mut command = fixture_command("noisy", output)?;
    command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    let result = supervise(command, Duration::from_millis(100), requests);
    if let Some(stopper) = stopper {
        stopper.join().map_err(|_| "stopper panicked")??;
    }
    fs::write(output.join("result"), format!("{result:?}"))?;
    Ok(if matches!(result, Err(Error::Control(_))) {
        0
    } else {
        93
    })
}

fn worker(mode: &str, output: &Path) -> TestResult<i32> {
    fs::write(output.join("at-gate"), b"waiting")?;
    let mut gate = [0_u8; 1];
    io::stdin().read_exact(&mut gate)?;
    if gate != *b"G" {
        return Ok(91);
    }
    if mode == "hung" || mode == "exit-with-writer" {
        fixture_command("writer", output)?.spawn()?;
        wait_for(&output.join("writes"))?;
    }
    println!("fixture ready");
    fs::write(output.join("ready"), b"ready")?;
    if mode == "exit" || mode == "exit-with-writer" {
        return Ok(37);
    }
    if mode == "verbose" || mode == "noisy" {
        let count = if mode == "verbose" { 1024 } else { 4096 };
        for _ in 0..count {
            io::stdout().write_all(&[b'~'; 1024])?;
        }
        io::stdout().flush()?;
        if mode == "verbose" {
            io::stderr().write_all(b"final diagnostic\n")?;
            return Ok(37);
        }
        loop {
            thread::park();
        }
    }
    let mut stop = [0_u8; 1];
    io::stdin().read_exact(&mut stop)?;
    if mode == "graceful" && stop == *b"S" {
        fs::write(output.join("complete"), b"done")?;
        return Ok(JOINED_EXIT_CODE);
    }
    loop {
        thread::park();
    }
}

pub(super) fn fixture_output() -> io::Result<TempDir> {
    let root = std::env::current_dir()?
        .join("target")
        .join("shutdown-tests");
    fs::create_dir_all(&root)?;
    tempfile::tempdir_in(root)
}

pub(super) fn fixture_command(mode: &str, output: &Path) -> io::Result<Command> {
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
