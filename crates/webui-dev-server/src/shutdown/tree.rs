// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{platform::Scope, Error, CONFIRMATION, STOPPING_POLL};
use std::io::{self, Write};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::thread;
use std::time::Instant;

pub(super) struct Completion {
    pub(super) status: ExitStatus,
    pub(super) descendants: bool,
}

pub(super) struct Tree {
    child: Child,
    pub(super) control: Option<ChildStdin>,
    scope: Scope,
    cleanup_attempted: bool,
}

impl Tree {
    pub(super) fn spawn(command: Command) -> Result<Self, Error> {
        Self::spawn_attached(command, Scope::attach)
    }

    fn spawn_attached(
        mut command: Command,
        attach: impl FnOnce(&mut Scope, &Child) -> io::Result<()>,
    ) -> Result<Self, Error> {
        let mut scope = Scope::new().map_err(Error::Startup)?;
        scope.configure(&mut command);
        let mut child = command
            .stdin(Stdio::piped())
            .spawn()
            .map_err(Error::Startup)?;
        if let Err(attach_error) = attach(&mut scope, &child) {
            // Still gated: kill only the direct child, never an unowned scope.
            cleanup_gated(&mut child)?;
            return Err(Error::Startup(attach_error));
        }
        Ok(Self {
            control: child.stdin.take(),
            child,
            scope,
            cleanup_attempted: false,
        })
    }

    pub(super) fn release(&mut self) -> io::Result<()> {
        self.control
            .as_mut()
            .ok_or_else(|| io::Error::other("missing worker startup pipe"))?
            .write_all(b"G")
    }

    pub(super) fn request_stop(&mut self) -> io::Result<()> {
        let Some(mut control) = self.control.take() else {
            return Err(io::Error::other("shutdown control pipe is closed"));
        };
        match control.write_all(b"S") {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe && self.root_exited()? => {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn root_exited(&self) -> io::Result<bool> {
        self.scope.root_exited(&self.child)
    }

    pub(super) fn finish(&mut self) -> Result<Completion, Error> {
        self.cleanup_attempted = true;
        let deadline = Instant::now()
            .checked_add(CONFIRMATION)
            .ok_or(Error::InvalidPolicy)?;
        let descendants = self.scope.descendants_after_exit(&self.child);
        // Never reap a Unix leader before this final group kill: retaining its
        // PID prevents accidentally signaling a reused PID/process group.
        let termination = self.scope.terminate(&self.child);
        self.control.take();
        // Even a failed query/kill gets a checked confirmation attempt. Do not
        // short-circuit before cleanup or retry with an extra budget from Drop.
        let status = self.confirm(deadline)?;
        termination.map_err(Error::Supervision)?;
        Ok(Completion {
            status,
            descendants: descendants.map_err(Error::Supervision)?,
        })
    }

    fn confirm(&mut self, deadline: Instant) -> Result<ExitStatus, Error> {
        let mut status = None;
        loop {
            if status.is_none() {
                status = self.child.try_wait().map_err(Error::Supervision)?;
            }
            if let Some(status) = status {
                if self.scope.empty().map_err(Error::Supervision)? {
                    return Ok(status);
                }
            }
            if Instant::now() >= deadline {
                return Err(Error::UnconfirmedTermination);
            }
            thread::sleep(STOPPING_POLL);
        }
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        if !self.cleanup_attempted {
            if let Err(error) = self.finish() {
                // Presentation is deliberately after the entire cleanup attempt.
                eprintln!(
                    "Dev-server emergency cleanup failed: {error}; {}",
                    error.help()
                );
            }
        }
    }
}

fn cleanup_gated(child: &mut Child) -> Result<(), Error> {
    let deadline = Instant::now()
        .checked_add(CONFIRMATION)
        .ok_or(Error::InvalidPolicy)?;
    let killed = child.kill();
    child.stdin.take();
    loop {
        if child.try_wait().map_err(Error::Supervision)?.is_some() {
            // kill may race an already exited gated child; a reaped child is
            // definitive confirmation, regardless of that race's kill result.
            return Ok(());
        }
        if Instant::now() >= deadline {
            // Keep the checked kill result rather than pretending termination.
            return match killed {
                Err(error) => Err(Error::Supervision(error)),
                Ok(()) => Err(Error::UnconfirmedTermination),
            };
        }
        thread::sleep(STOPPING_POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shutdown::{CHILD_ENV, CHILD_VERSION};

    #[test]
    fn failed_attach_kills_and_reaps_the_unreleased_child() -> Result<(), Box<dyn std::error::Error>>
    {
        let output = crate::shutdown::tests::support::output()?;
        let mut command = crate::shutdown::tests::support::command("graceful", output.path())?;
        command.env(CHILD_ENV, CHILD_VERSION);
        let mut spawned = false;
        let result = Tree::spawn_attached(command, |_scope, child| {
            assert!(child.id() > 0);
            spawned = true;
            Err(io::Error::other("injected containment failure"))
        });
        assert!(spawned);
        assert!(matches!(result, Err(Error::Startup(_))));
        assert!(!output.path().join("root.ready").exists());
        Ok(())
    }
}
