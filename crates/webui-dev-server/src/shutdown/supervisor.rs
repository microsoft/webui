// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{
    tree::{Completion, Tree},
    validate_policy, Error, ForcedReason, CHILD_ENV, CHILD_VERSION, JOINED_EXIT_CODE, RUNNING_POLL,
    STOPPING_POLL,
};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError};
use std::thread;
use std::time::{Duration, Instant};

pub(super) struct StopHandle(SyncSender<()>);

impl StopHandle {
    pub(super) fn request(&self) -> bool {
        match self.0.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) => true,
            Err(TrySendError::Disconnected(())) => false,
        }
    }
}

pub(super) fn stop_channel() -> (StopHandle, Receiver<()>) {
    let (sender, receiver) = mpsc::sync_channel(2);
    (StopHandle(sender), receiver)
}

pub(super) fn supervise(
    mut command: Command,
    grace: Duration,
    requests: Receiver<()>,
) -> Result<i32, Error> {
    validate_policy(grace)?;
    command.env(CHILD_ENV, CHILD_VERSION);
    let mut tree = Tree::spawn(command)?;
    let monitored = tree
        .release()
        .map_err(Error::Supervision)
        .and_then(|()| monitor(&mut tree, grace, &requests));
    // Cleanup failures take priority: never hide an unconfirmed scope behind a
    // pipe, polling, or startup error, and never spend a second budget in Drop.
    let completion = tree.finish()?;
    let forced = monitored?;
    exit_code(completion, forced)
}

fn exit_code(completion: Completion, forced: Option<ForcedReason>) -> Result<i32, Error> {
    if let Some(reason) = forced {
        return Err(Error::Forced(reason));
    }
    match completion.status.code() {
        // Windows can count surviving job members after root exit. Unix cannot
        // distinguish them while retaining its zombie leader, so it trusts ONLY
        // our controlled CLI's post-join attestation, never arbitrary exit(0).
        Some(JOINED_EXIT_CODE) if !completion.descendants => Ok(0),
        Some(0 | JOINED_EXIT_CODE) | None => Err(Error::Forced(ForcedReason::ChildExited)),
        // Ordinary child failures have already been reported by the child.
        Some(code) => Ok(code),
    }
}

fn monitor(
    tree: &mut Tree,
    grace: Duration,
    requests: &Receiver<()>,
) -> Result<Option<ForcedReason>, Error> {
    let mut deadline = None;
    loop {
        #[cfg(unix)]
        super::unix_signals::check_forwarding().map_err(Error::Supervision)?;
        if expired(deadline) {
            return Ok(Some(ForcedReason::Deadline));
        }
        match requests.try_recv() {
            Ok(()) => {
                if request_stop(tree, grace, &mut deadline)? {
                    return Ok(Some(ForcedReason::RepeatedRequest));
                }
                // Drain queued requests before accepting even a successful exit.
                continue;
            }
            Err(TryRecvError::Disconnected) if deadline.is_none() => {
                request_stop(tree, grace, &mut deadline)?;
                continue;
            }
            Err(_) => {}
        }
        if tree.root_exited().map_err(Error::Supervision)? {
            if expired(deadline) {
                return Ok(Some(ForcedReason::Deadline));
            }
            // Observe a repeated request racing with the root status probe.
            if deadline.is_some() && requests.try_recv().is_ok() {
                return Ok(Some(ForcedReason::RepeatedRequest));
            }
            return Ok(None);
        }
        let pause = deadline.map_or(RUNNING_POLL, |limit: Instant| {
            limit
                .saturating_duration_since(Instant::now())
                .min(STOPPING_POLL)
        });
        match requests.recv_timeout(pause) {
            Ok(()) => {
                if request_stop(tree, grace, &mut deadline)? {
                    return Ok(Some(ForcedReason::RepeatedRequest));
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                if deadline.is_none() {
                    request_stop(tree, grace, &mut deadline)?;
                } else {
                    thread::sleep(pause);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

fn expired(deadline: Option<Instant>) -> bool {
    deadline.is_some_and(|limit| Instant::now() >= limit)
}

fn request_stop(
    tree: &mut Tree,
    grace: Duration,
    deadline: &mut Option<Instant>,
) -> Result<bool, Error> {
    if deadline.is_some() {
        return Ok(true);
    }
    *deadline = Some(
        Instant::now()
            .checked_add(grace)
            .ok_or(Error::InvalidPolicy)?,
    );
    tree.request_stop().map_err(Error::Supervision)?;
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::ExitStatus;

    fn status(code: u8) -> ExitStatus {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            ExitStatus::from_raw(i32::from(code) << 8)
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::ExitStatusExt;
            ExitStatus::from_raw(u32::from(code))
        }
    }

    #[test]
    fn escalation_wins_over_racing_zero_or_join_attestation() {
        for code in [0, 125, 23] {
            for reason in [ForcedReason::Deadline, ForcedReason::RepeatedRequest] {
                let result = exit_code(
                    Completion {
                        status: status(code),
                        descendants: false,
                    },
                    Some(reason),
                );
                assert!(matches!(result, Err(Error::Forced(actual)) if actual == reason));
            }
        }
    }

    #[test]
    fn stop_queue_preserves_two_requests_without_blocking() {
        let (stop, requests) = stop_channel();
        assert!(stop.request());
        assert!(stop.request());
        assert!(stop.request());
        assert_eq!(requests.try_recv(), Ok(()));
        assert_eq!(requests.try_recv(), Ok(()));
        assert_eq!(requests.try_recv(), Err(TryRecvError::Empty));
        drop(requests);
        assert!(!stop.request());
    }
}
