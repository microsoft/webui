// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Opt-in, process-bounded dev-server shutdown.

#[cfg(unix)]
#[path = "unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "windows.rs"]
mod platform;

mod notification;
mod stdin_control;
mod task;
mod worker;

use actix_web::dev::Server;
use notification::{stop_channel, Requests};
use std::io::{self, Read};
use std::num::NonZeroU64;
use std::process::{Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};
use stdin_control::StdinControl;
use tokio::sync::watch;
use worker::Worker;

const CHILD_ENV: &str = "_WEBUI_DEV_SERVER_CHILD";
const CHILD_VERSION: &str = "bounded-v1";
const CONFIRMATION: Duration = Duration::from_secs(2);
const STOPPING_POLL: Duration = Duration::from_millis(10);

/// Private success status returned after HTTP and rebuild teardown completes.
pub const JOINED_EXIT_CODE: i32 = 125;

/// Why a bounded shutdown required forced process termination.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ForcedReason {
    /// The graceful shutdown deadline expired.
    #[error("the graceful shutdown deadline expired")]
    Deadline,
    /// A second stop request bypassed the remaining grace period.
    #[error("a repeated shutdown request required immediate termination")]
    RepeatedRequest,
}

/// Bounded shutdown failures.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The timeout cannot be represented by the monotonic clock.
    #[error("invalid dev-server shutdown timeout")]
    InvalidPolicy,
    /// The child process or its containment scope could not be started.
    #[error("could not start a contained dev-server worker")]
    Startup(#[source] io::Error),
    /// The process-wide shutdown handler could not be installed.
    #[error("could not install the dev-server shutdown signal handler")]
    Signal(#[source] ctrlc::Error),
    /// Parent-child shutdown control failed.
    #[error("dev-server shutdown control failed")]
    Control(#[source] io::Error),
    /// The HTTP server failed.
    #[error("the dev HTTP server failed")]
    Http(#[source] io::Error),
    /// Forced termination did not complete within the confirmation window.
    #[error("dev-server termination was not confirmed within two seconds")]
    UnconfirmedTermination,
    /// The child was terminated by a signal or equivalent platform event.
    #[error("the dev-server worker exited without a status code")]
    ChildExited,
    /// The grace period was bypassed or expired.
    #[error("dev-server shutdown was forced: {0}")]
    Forced(ForcedReason),
}

/// Whether this invocation serves directly, serves under supervision, or exits.
pub enum Mode {
    /// No shutdown policy; retain the existing in-process behavior.
    Direct,
    /// A contained worker whose HTTP server must disable Actix signals.
    Child(Control),
    /// The supervising parent should exit with this code.
    Supervisor(i32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ControlState {
    Running,
    Stop,
    Failed(io::ErrorKind),
}

/// Child-side bridge from the supervisor pipe to Actix shutdown.
pub struct Control(watch::Receiver<ControlState>);

impl Control {
    /// Wait for a supervisor stop or report loss of its control channel.
    ///
    /// Application orchestrators use this to stop native hosts gracefully
    /// before releasing their request-scoped rendering resources.
    pub async fn shutdown_requested(&mut self) -> Result<(), Error> {
        match self.stop_state().await {
            ControlState::Stop => Ok(()),
            ControlState::Failed(kind) => Err(Error::Control(io::Error::new(
                kind,
                "the supervisor control pipe closed",
            ))),
            ControlState::Running => Err(Error::Control(io::Error::other(
                "invalid supervisor stop state",
            ))),
        }
    }

    /// Interrupt the gated worker's owned process group during a build.
    ///
    /// The worker must install signal handling before calling this method;
    /// the signal includes the worker so it can observe and join cancellation.
    /// The supervisor's containment deadline remains the final force-stop bound.
    pub fn interrupt_build(&self) -> Result<(), Error> {
        platform::interrupt_current_group().map_err(Error::Control)
    }

    /// Stop active HTTP connections when the supervisor requests shutdown.
    ///
    /// The caller must still drop its watcher and join active rebuild work after
    /// this method returns.
    pub async fn serve(mut self, server: Server) -> Result<(), Error> {
        let handle = server.handle();
        tokio::pin!(server);
        tokio::select! {
            biased;
            state = self.stop_state() => {
                let (_, result) =
                    futures_util::future::join(handle.stop(false), &mut server).await;
                result.map_err(Error::Http)?;
                match state {
                    ControlState::Stop => Ok(()),
                    ControlState::Failed(kind) => Err(Error::Control(io::Error::new(
                        kind,
                        "the supervisor control pipe closed",
                    ))),
                    ControlState::Running => Ok(()),
                }
            }
            result = &mut server => result.map_err(Error::Http),
        }
    }

    async fn stop_state(&mut self) -> ControlState {
        loop {
            let state = *self.0.borrow_and_update();
            if state != ControlState::Running {
                return state;
            }
            if self.0.changed().await.is_err() {
                return ControlState::Failed(io::ErrorKind::UnexpectedEof);
            }
        }
    }
}

/// Enter the private startup gate or supervise this invocation of the CLI.
///
/// Call this before output, builds, watchers, or application threads. Without a
/// timeout it installs no handler and spawns no child.
///
/// # Errors
///
/// Returns an error when signal registration, child startup, control, or forced
/// termination fails.
pub fn prepare(timeout: Option<NonZeroU64>) -> Result<Mode, Error> {
    prepare_with_stdin(timeout, false)
}

/// Enter bounded supervision, optionally accepting machine stop requests.
///
/// Like [`prepare`], call this once at process entry, before output or work.
/// With a timeout and `enabled`, the supervising parent starts one stdin reader.
/// Its first ASCII `S` byte or EOF requests graceful stop; another first byte
/// reports a control error after cleanup. Only the first byte is interpreted.
/// Without a timeout, stdin is untouched and execution remains direct.
///
/// The private child gate is checked first, so a worker never starts this
/// external reader on its private startup/shutdown pipe. The reader is
/// process-scoped: an already-completed worker never waits for controller EOF.
///
/// # Errors
///
/// Returns the same failures as [`prepare`], plus stdin reader startup, read,
/// or malformed-control errors when machine control is enabled.
pub fn prepare_with_stdin(timeout: Option<NonZeroU64>, enabled: bool) -> Result<Mode, Error> {
    if let Some(control) = child_gate()? {
        return Ok(Mode::Child(control));
    }
    let Some(seconds) = timeout else {
        return Ok(Mode::Direct);
    };
    let grace = Duration::from_secs(seconds.get());
    validate_policy(grace)?;
    let (stop, requests) = stop_channel();
    ctrlc::set_handler(move || {
        let _ = stop.request();
    })
    .map_err(Error::Signal)?;
    let executable = std::env::current_exe().map_err(Error::Startup)?;
    let mut command = Command::new(executable);
    command.args(std::env::args_os().skip(1));
    supervise_with_stdin(command, grace, requests, enabled).map(Mode::Supervisor)
}

fn validate_policy(grace: Duration) -> Result<(), Error> {
    if Instant::now().checked_add(grace).is_none() {
        return Err(Error::InvalidPolicy);
    }
    Ok(())
}

fn child_gate() -> Result<Option<Control>, Error> {
    let Some(version) = std::env::var_os(CHILD_ENV) else {
        return Ok(None);
    };
    std::env::remove_var(CHILD_ENV);
    if version != CHILD_VERSION {
        return Err(Error::Startup(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported child startup protocol",
        )));
    }
    let mut command = [0_u8; 1];
    io::stdin()
        .read_exact(&mut command)
        .map_err(Error::Startup)?;
    if command != *b"G" {
        return Err(Error::Startup(io::Error::new(
            io::ErrorKind::InvalidData,
            "missing child startup permission",
        )));
    }
    let (sender, receiver) = watch::channel(ControlState::Running);
    thread::Builder::new()
        .name("webui-shutdown-control".to_owned())
        .spawn(move || {
            let mut command = [0_u8; 1];
            let state = match io::stdin().read_exact(&mut command) {
                Ok(()) if command == *b"S" => ControlState::Stop,
                Ok(()) => ControlState::Failed(io::ErrorKind::InvalidData),
                Err(error) => ControlState::Failed(error.kind()),
            };
            let _ = sender.send(state);
        })
        .map_err(Error::Control)?;
    Ok(Some(Control(receiver)))
}

#[cfg(test)]
fn supervise(command: Command, grace: Duration, requests: Requests) -> Result<i32, Error> {
    supervise_with_stdin(command, grace, requests, false)
}

fn supervise_with_stdin(
    mut command: Command,
    grace: Duration,
    requests: Requests,
    enabled: bool,
) -> Result<i32, Error> {
    validate_policy(grace)?;
    command.env(CHILD_ENV, CHILD_VERSION);
    let mut worker = Worker::spawn(command, requests.notification.clone())?;
    let mut input = match StdinControl::start(enabled, requests.notification.clone()) {
        Ok(input) => input,
        Err(error) => return Err(worker.abort(error)),
    };
    let result = worker
        .release()
        .and_then(|()| supervise_running(&mut worker, grace, &requests, &mut input));
    let result = result.map_err(|error| worker.abort(error));
    // Include input already reported during worker/scope cleanup, too.
    input.poll(&requests);
    input.resolve(result)
}

fn supervise_running(
    worker: &mut Worker,
    grace: Duration,
    requests: &Requests,
    input: &mut StdinControl,
) -> Result<i32, Error> {
    let mut deadline = None;
    loop {
        input.poll(requests);
        if worker.exited()? {
            return exit_code(worker.finish()?);
        }
        worker.check_relays()?;
        if requests.count() >= 2 {
            return Err(Error::Forced(ForcedReason::RepeatedRequest));
        }
        if deadline.is_some_and(|limit| Instant::now() >= limit) {
            return Err(Error::Forced(ForcedReason::Deadline));
        }
        if deadline.is_none() && (requests.count() != 0 || requests.disconnected()) {
            worker.request_stop()?;
            deadline = Some(
                Instant::now()
                    .checked_add(grace)
                    .ok_or(Error::InvalidPolicy)?,
            );
        }
        requests.wait(deadline)?;
    }
}

fn exit_code(status: ExitStatus) -> Result<i32, Error> {
    match status.code() {
        Some(JOINED_EXIT_CODE) => Ok(0),
        Some(code) => Ok(code),
        None => Err(Error::ChildExited),
    }
}

#[cfg(test)]
mod tests;
