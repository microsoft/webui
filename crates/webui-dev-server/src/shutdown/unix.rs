// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(unsafe_code)]

use std::io;
use std::mem::zeroed;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};

pub(super) struct Scope {
    group: libc::pid_t,
}

impl Scope {
    pub(super) fn new() -> io::Result<Self> {
        Ok(Self { group: 0 })
    }

    pub(super) fn configure(&self, command: &mut Command) {
        command.process_group(0);
    }

    pub(super) fn attach(&mut self, child: &Child) -> io::Result<()> {
        let group = libc::pid_t::try_from(child.id()).map_err(io::Error::other)?;
        if group <= 0 {
            return Err(io::Error::other("invalid owned process group"));
        }
        // SAFETY: child is our unreaped direct child; querying its group does
        // not modify any process. Command::process_group established it pre-exec.
        let actual = unsafe { libc::getpgid(group) };
        if actual == -1 {
            return Err(io::Error::last_os_error());
        }
        if actual != group {
            return Err(io::Error::other("worker is not in its owned process group"));
        }
        self.group = group;
        Ok(())
    }

    pub(super) fn root_exited(&self, child: &Child) -> io::Result<bool> {
        // SAFETY: siginfo_t is a C output structure permitting zero initialization.
        let mut info: libc::siginfo_t = unsafe { zeroed() };
        // SAFETY: The PID belongs to our unreaped direct child. WNOWAIT keeps it
        // reserved until the final group kill, unlike Child::try_wait.
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                child.id(),
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: waitid initialized info and si_pid is valid for its result.
        Ok(unsafe { info.si_pid() } != 0)
    }

    pub(super) fn descendants_after_exit(&self, _child: &Child) -> io::Result<bool> {
        // POSIX cannot distinguish other members from the retained zombie
        // leader. The caller requires JOINED_EXIT_CODE, not exit(0), as its
        // narrow controlled-CLI join attestation. Final kill/empty checks still
        // run in every case, including that attestation.
        Ok(false)
    }

    pub(super) fn terminate(&self) -> io::Result<()> {
        if self.group <= 0 {
            return Err(io::Error::other("refusing an unowned process-group signal"));
        }
        // SAFETY: The owned leader has not been reaped, preventing PGID reuse.
        if unsafe { libc::kill(-self.group, libc::SIGKILL) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(error)
        }
    }

    pub(super) fn empty(&self) -> io::Result<bool> {
        // SAFETY: Signal zero only probes existence. No signals are sent after
        // the root was reaped; an ambiguous reused PGID fails closed.
        if unsafe { libc::kill(-self.group, 0) } == 0 {
            return Ok(false);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(true)
        } else {
            Err(error)
        }
    }
}
