// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(unsafe_code)]

use std::io;
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

    pub(super) fn terminate(&self, _child: &Child) -> io::Result<()> {
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
            Err(error)
        }
    }
}
