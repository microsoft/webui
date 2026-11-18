// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(unsafe_code)]

use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};
use std::time::Instant;

pub(super) fn wait_for_exit(child: &mut Child) -> io::Result<()> {
    // Do not reap yet: even an exited worker must pin its PID/group identity
    // until the supervisor has killed any remaining descendants.
    loop {
        // SAFETY: siginfo_t permits zero initialization and waitid writes it.
        let mut info = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
        // SAFETY: child is an owned, unreaped direct child; info is writable.
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                child.id(),
                &mut info,
                libc::WEXITED | libc::WNOWAIT,
            )
        };
        if result == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

pub(super) fn interrupt_current_group() -> io::Result<()> {
    let process = libc::pid_t::try_from(std::process::id()).map_err(io::Error::other)?;
    // SAFETY: getpgrp has no preconditions. Only the worker group established
    // before its startup gate may be signalled, never the caller's terminal group.
    if unsafe { libc::getpgrp() } != process {
        return Err(io::Error::other(
            "worker is not its owned process-group leader",
        ));
    }
    // SAFETY: The checked process group is owned by this gated worker.
    if unsafe { libc::kill(-process, libc::SIGTERM) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

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
        // SAFETY: child is our direct child and process_group established this
        // group before exec.
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

    pub(super) fn terminate(&self, _deadline: Instant) -> io::Result<()> {
        if self.group <= 0 {
            return Err(io::Error::other("refusing an unowned process-group signal"));
        }
        // SAFETY: The process group was created for this direct child.
        if unsafe { libc::kill(-self.group, libc::SIGKILL) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            // Darwin reports EPERM, not ESRCH, for a group containing only its
            // unreaped zombie leader. Do not ignore other permission failures.
            #[cfg(target_os = "macos")]
            if error.raw_os_error() == Some(libc::EPERM) {
                loop {
                    if self.only_zombie_leader()? {
                        return Ok(());
                    }
                    // A concurrently exiting leader can already reject signals
                    // before waitid reports it as a zombie. Keep its PID pinned
                    // and allow that transition only within the shared budget.
                    let remaining = _deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    std::thread::sleep(remaining.min(super::STOPPING_POLL));
                }
            }
            Err(error)
        }
    }

    #[cfg(target_os = "macos")]
    fn only_zombie_leader(&self) -> io::Result<bool> {
        const PROC_PGRP_ONLY: u32 = 2;
        // Two entries suffice: any second member must preserve the error.
        let mut members = [0 as libc::pid_t; 2];
        // SAFETY: proc_listpids writes at most the supplied buffer size.
        let bytes = unsafe {
            libc::proc_listpids(
                PROC_PGRP_ONLY,
                u32::try_from(self.group).map_err(io::Error::other)?,
                members.as_mut_ptr().cast(),
                i32::try_from(std::mem::size_of_val(&members)).map_err(io::Error::other)?,
            )
        };
        if bytes < 0 {
            return Err(io::Error::last_os_error());
        }
        if bytes != i32::try_from(std::mem::size_of::<libc::pid_t>()).map_err(io::Error::other)?
            || members[0] != self.group
        {
            return Ok(false);
        }
        // SAFETY: The zeroed siginfo is writable and WNOHANG cannot block.
        let mut info = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
        // SAFETY: The group leader is still our owned, unreaped direct child.
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                u32::try_from(self.group).map_err(io::Error::other)?,
                &mut info,
                libc::WEXITED | libc::WNOWAIT | libc::WNOHANG,
            )
        };
        if result == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(info.si_pid == self.group)
    }
}
