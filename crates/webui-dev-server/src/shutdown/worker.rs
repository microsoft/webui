// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::notification::Notification;
use super::task::Task;
use super::{platform, Error, CONFIRMATION, STOPPING_POLL};
use std::io::{self, Write};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

type ChildExit = Result<(Child, io::Result<()>), mpsc::RecvError>;

pub(super) struct Worker {
    // The waiter exclusively owns Child while blocked. Scope termination never
    // needs that handle or a mutex held by the waiter.
    child: Option<Child>,
    waiter: Option<Task<ChildExit>>,
    control: Option<ChildStdin>,
    scope: platform::Scope,
    attached: bool,
    relays: Vec<Task<io::Result<u64>>>,
    finished: bool,
}

impl Worker {
    pub(super) fn spawn(mut command: Command, notification: Notification) -> Result<Self, Error> {
        let scope = platform::Scope::new().map_err(Error::Startup)?;
        scope.configure(&mut command);
        #[cfg(unix)]
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command
            .stdin(Stdio::piped())
            .spawn()
            .map_err(Error::Startup)?;
        let control = child.stdin.take();
        let mut worker = Self {
            child: Some(child),
            waiter: None,
            control,
            scope,
            attached: false,
            relays: Vec::with_capacity(2),
            finished: false,
        };
        if let Err(error) = worker.start(notification) {
            return Err(worker.abort(Error::Startup(error)));
        }
        Ok(worker)
    }

    fn start(&mut self, notification: Notification) -> io::Result<()> {
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| io::Error::other("missing gated worker"))?;
        self.scope.attach(child)?;
        self.attached = true;
        // Containment, both relays, and the exit waiter must exist before G.
        #[cfg(unix)]
        {
            let mut stdout = child
                .stdout
                .take()
                .ok_or_else(|| io::Error::other("missing supervised stdout pipe"))?;
            self.relays.push(Task::spawn(
                "webui-shutdown-stdout",
                notification.clone(),
                move || io::copy(&mut stdout, &mut io::stdout()),
            )?);
            let mut stderr = child
                .stderr
                .take()
                .ok_or_else(|| io::Error::other("missing supervised stderr pipe"))?;
            self.relays.push(Task::spawn(
                "webui-shutdown-stderr",
                notification.clone(),
                move || io::copy(&mut stderr, &mut io::stderr()),
            )?);
        }
        let (sender, receiver) = mpsc::sync_channel::<Child>(1);
        self.waiter = Some(Task::spawn(
            "webui-shutdown-exit",
            notification,
            move || {
                receiver.recv().map(|mut child| {
                    let result = platform::wait_for_exit(&mut child);
                    (child, result)
                })
            },
        )?);
        let child = self
            .child
            .take()
            .ok_or_else(|| io::Error::other("missing worker for exit waiter"))?;
        if let Err(error) = sender.send(child) {
            self.child = Some(error.0);
            return Err(io::Error::other("exit waiter rejected the worker"));
        }
        Ok(())
    }

    pub(super) fn release(&mut self) -> Result<(), Error> {
        self.write_control(b'G')
    }

    pub(super) fn request_stop(&mut self) -> Result<(), Error> {
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

    pub(super) fn exited(&mut self) -> Result<bool, Error> {
        if let Some(waiter) = &mut self.waiter {
            if let Some(result) = waiter.take_result().map_err(Error::Control)? {
                let (child, result) =
                    result.map_err(|error| Error::Control(io::Error::other(error)))?;
                self.child = Some(child);
                result.map_err(Error::Control)?;
            }
        }
        Ok(self.child.is_some())
    }

    pub(super) fn check_relays(&mut self) -> Result<(), Error> {
        for relay in &mut self.relays {
            if let Some(result) = relay.take_result().map_err(Error::Control)? {
                result.map_err(|error| relay.failure(error))?;
            }
        }
        Ok(())
    }

    pub(super) fn finish(&mut self) -> Result<ExitStatus, Error> {
        // Every explicit path cleans up once, with one shared confirmation
        // budget. Drop must not silently add another confirmation window.
        if self.finished {
            return Err(Error::Control(io::Error::new(
                io::ErrorKind::InvalidInput,
                "worker teardown was already attempted",
            )));
        }
        self.finished = true;
        let deadline = Instant::now() + CONFIRMATION;
        let mut failure = None;
        record(&mut failure, self.kill_scope(deadline));
        self.control.take();
        let status = match self.reap_until(deadline) {
            Ok(status) => Some(status),
            Err(error) => {
                record(&mut failure, Err(error));
                None
            }
        };
        if let Some(waiter) = &mut self.waiter {
            record(&mut failure, waiter.join_until(deadline));
        }
        for relay in &mut self.relays {
            record(&mut failure, relay.join_until(deadline));
            let result = relay.take_result().and_then(|value| value.transpose());
            record(
                &mut failure,
                result.map(|_| ()).map_err(|error| relay.failure(error)),
            );
        }
        if let Some(error) = failure {
            return Err(error);
        }
        status.ok_or(Error::UnconfirmedTermination)
    }

    fn kill_scope(&mut self, deadline: Instant) -> Result<(), Error> {
        if self.attached {
            // On Unix the waiter observes without reaping: its PID still pins
            // the verified group identity until after this signal.
            self.scope.terminate(deadline).map_err(Error::Control)
        } else if let Some(child) = self.child.as_mut() {
            // Failed attachment means G was never sent, so there can be no
            // descendants; only this direct child may be killed.
            if child.try_wait().map_err(Error::Control)?.is_none() {
                child.kill().map_err(Error::Control)?;
            }
            Ok(())
        } else {
            Err(Error::Control(io::Error::other("missing gated worker")))
        }
    }

    fn reap_until(&mut self, deadline: Instant) -> Result<ExitStatus, Error> {
        let mut failure = None;
        loop {
            // A failed observation can still return ownership of Child. Always
            // attempt to reap it rather than letting that error skip cleanup.
            record(&mut failure, self.exited().map(|_| ()));
            if let Some(child) = &mut self.child {
                if let Some(status) = child.try_wait().map_err(Error::Control)? {
                    return match failure {
                        Some(error) => Err(error),
                        None => Ok(status),
                    };
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(match failure {
                    Some(error) => combined(error, Error::UnconfirmedTermination),
                    None => Error::UnconfirmedTermination,
                });
            }
            thread::sleep(remaining.min(STOPPING_POLL));
        }
    }

    pub(super) fn abort(&mut self, error: Error) -> Error {
        if self.finished {
            return error;
        }
        match self.finish() {
            Ok(_) => error,
            Err(cleanup) => combined(error, cleanup),
        }
    }
}

fn record(failure: &mut Option<Error>, result: Result<(), Error>) {
    if let Err(error) = result {
        *failure = Some(match failure.take() {
            Some(previous) => combined(previous, error),
            None => error,
        });
    }
}

#[cold]
pub(super) fn combined(primary: Error, cleanup: Error) -> Error {
    Error::Control(io::Error::other(format!(
        "{primary}: {}; teardown failed: {cleanup}: {}",
        std::error::Error::source(&primary)
            .map(ToString::to_string)
            .unwrap_or_default(),
        std::error::Error::source(&cleanup)
            .map(ToString::to_string)
            .unwrap_or_default(),
    )))
}

impl Drop for Worker {
    fn drop(&mut self) {
        if !self.finished {
            // Explicit error paths use abort(), which reports cleanup failures.
            // This is only the last-resort guard for unwinding callers.
            let _ = self.finish();
        }
    }
}
