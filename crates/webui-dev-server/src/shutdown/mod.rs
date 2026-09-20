// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Opt-in, process-bounded dev-server shutdown.

#[cfg(unix)]
#[path = "unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "windows.rs"]
mod platform;

use actix_web::dev::Server;
use std::io::{self, Read, Write};
use std::num::NonZeroU64;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::thread;
use std::time::{Duration, Instant};
use tokio::sync::watch;

const CHILD_ENV: &str = "_WEBUI_DEV_SERVER_CHILD";
const CHILD_VERSION: &str = "bounded-v1";
const CONFIRMATION: Duration = Duration::from_secs(2);
const RUNNING_POLL: Duration = Duration::from_millis(100);
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
    supervise(command, grace, requests).map(Mode::Supervisor)
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

struct StopHandle(SyncSender<()>);

impl StopHandle {
    fn request(&self) -> bool {
        match self.0.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) => true,
            Err(TrySendError::Disconnected(())) => false,
        }
    }
}

fn stop_channel() -> (StopHandle, Receiver<()>) {
    let (sender, receiver) = mpsc::sync_channel(2);
    (StopHandle(sender), receiver)
}

fn supervise(mut command: Command, grace: Duration, requests: Receiver<()>) -> Result<i32, Error> {
    validate_policy(grace)?;
    command.env(CHILD_ENV, CHILD_VERSION);
    let mut worker = Worker::spawn(command)?;
    worker.release()?;
    let mut deadline = None;
    loop {
        if let Some(status) = worker.try_wait()? {
            worker.finished = true;
            return exit_code(status);
        }
        if deadline.is_some_and(|limit| Instant::now() >= limit) {
            worker.terminate()?;
            return Err(Error::Forced(ForcedReason::Deadline));
        }
        let pause = deadline.map_or(RUNNING_POLL, |limit: Instant| {
            limit
                .saturating_duration_since(Instant::now())
                .min(STOPPING_POLL)
        });
        match requests.recv_timeout(pause) {
            Ok(()) if deadline.is_some() => {
                worker.terminate()?;
                return Err(Error::Forced(ForcedReason::RepeatedRequest));
            }
            Ok(()) => {
                worker.request_stop()?;
                deadline = Some(
                    Instant::now()
                        .checked_add(grace)
                        .ok_or(Error::InvalidPolicy)?,
                );
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) if deadline.is_none() => {
                worker.request_stop()?;
                deadline = Some(
                    Instant::now()
                        .checked_add(grace)
                        .ok_or(Error::InvalidPolicy)?,
                );
            }
            Err(RecvTimeoutError::Disconnected) => thread::sleep(pause),
        }
    }
}

fn exit_code(status: ExitStatus) -> Result<i32, Error> {
    match status.code() {
        Some(JOINED_EXIT_CODE) => Ok(0),
        Some(code) => Ok(code),
        None => Err(Error::ChildExited),
    }
}

struct Worker {
    child: Child,
    control: Option<ChildStdin>,
    scope: platform::Scope,
    output_relays: Vec<thread::JoinHandle<()>>,
    finished: bool,
}

impl Worker {
    fn spawn(mut command: Command) -> Result<Self, Error> {
        let mut scope = platform::Scope::new().map_err(Error::Startup)?;
        scope.configure(&mut command);
        #[cfg(unix)]
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command
            .stdin(Stdio::piped())
            .spawn()
            .map_err(Error::Startup)?;
        if let Err(error) = scope.attach(&child) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Startup(error));
        }
        let output_relays = match start_output_relays(&mut child) {
            Ok(relays) => relays,
            Err(error) => {
                let _ = scope.terminate(&child);
                let _ = child.wait();
                return Err(Error::Startup(error));
            }
        };
        let control = child.stdin.take();
        Ok(Self {
            child,
            control,
            scope,
            output_relays,
            finished: false,
        })
    }

    fn release(&mut self) -> Result<(), Error> {
        self.write_control(b'G')
    }

    fn request_stop(&mut self) -> Result<(), Error> {
        self.write_control(b'S')?;
        self.control.take();
        Ok(())
    }

    fn write_control(&mut self, byte: u8) -> Result<(), Error> {
        self.control
            .as_mut()
            .ok_or_else(|| {
                Error::Control(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "shutdown control pipe is closed",
                ))
            })?
            .write_all(&[byte])
            .map_err(Error::Control)
    }

    fn try_wait(&mut self) -> Result<Option<ExitStatus>, Error> {
        self.child.try_wait().map_err(Error::Control)
    }

    fn terminate(&mut self) -> Result<(), Error> {
        self.scope.terminate(&self.child).map_err(Error::Control)?;
        let deadline = Instant::now()
            .checked_add(CONFIRMATION)
            .ok_or(Error::InvalidPolicy)?;
        loop {
            if self.try_wait()?.is_some() {
                self.finished = true;
                self.join_output_relays();
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(Error::UnconfirmedTermination);
            }
            thread::sleep(STOPPING_POLL);
        }
    }

    fn join_output_relays(&mut self) {
        for relay in self.output_relays.drain(..) {
            let _ = relay.join();
        }
    }
}

#[cfg(unix)]
fn start_output_relays(child: &mut Child) -> io::Result<Vec<thread::JoinHandle<()>>> {
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("missing supervised stdout pipe"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("missing supervised stderr pipe"))?;
    Ok(vec![
        thread::Builder::new()
            .name("webui-shutdown-stdout".to_owned())
            .spawn(move || {
                let _ = io::copy(&mut stdout, &mut io::stdout());
            })?,
        thread::Builder::new()
            .name("webui-shutdown-stderr".to_owned())
            .spawn(move || {
                let _ = io::copy(&mut stderr, &mut io::stderr());
            })?,
    ])
}

#[cfg(windows)]
fn start_output_relays(_child: &mut Child) -> io::Result<Vec<thread::JoinHandle<()>>> {
    Ok(Vec::new())
}

impl Drop for Worker {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.terminate();
        }
    }
}

#[cfg(test)]
mod tests;
