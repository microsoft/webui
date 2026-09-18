// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::io;

/// Why a bounded serve invocation did not finish its controlled teardown.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ForcedReason {
    /// HTTP shutdown or the active rebuild exceeded the configured grace period.
    #[error("the graceful shutdown deadline expired")]
    Deadline,
    /// A second stop request bypassed the remaining grace period.
    #[error("a repeated shutdown request required immediate termination")]
    RepeatedRequest,
    /// The child exited without completed teardown, or left Windows job members.
    #[error("the worker exited without confirmed completed teardown")]
    ChildExited,
}

/// Plain, structured shutdown failures. An error never licenses output cleanup.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The timeout is zero or cannot be represented by the monotonic clock.
    #[error("invalid dev-server shutdown timeout")]
    InvalidPolicy,
    /// Startup or containment failed before the child's startup gate opened.
    #[error("could not start a contained dev-server worker")]
    Startup(#[source] io::Error),
    /// An operating-system or parent-side control operation failed.
    #[error("dev-server process supervision failed")]
    Supervision(#[source] io::Error),
    /// The owned process scope was not confirmed empty within two seconds.
    #[error("dev-server process-tree termination was not confirmed within two seconds")]
    UnconfirmedTermination,
    /// The parent could not install its process-wide shutdown signal handler.
    #[error("could not install the dev-server shutdown signal handler")]
    Signal(#[source] ctrlc::Error),
    /// The child's private control pipe closed or received invalid data.
    #[error("the dev-server supervisor connection failed; HTTP shutdown was requested")]
    Control(#[source] io::Error),
    /// HTTP serving failed; the caller must still drop its watcher and join work.
    #[error("the dev HTTP server failed")]
    Http(#[source] io::Error),
    /// Teardown required forced termination, even if the child raced to success.
    #[error("dev-server shutdown was forced: {0}")]
    Forced(ForcedReason),
}

impl Error {
    /// Stable machine-readable code for CLI JSON diagnostics.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidPolicy => "shutdown-invalid-policy",
            Self::Startup(_) => "shutdown-startup",
            Self::Supervision(_) => "shutdown-supervision",
            Self::UnconfirmedTermination => "shutdown-unconfirmed-termination",
            Self::Signal(_) => "shutdown-signal",
            Self::Control(_) => "shutdown-control",
            Self::Http(_) => "shutdown-http",
            Self::Forced(_) => "shutdown-forced",
        }
    }

    /// Actionable, color-free advice for terminal and JSON consumers.
    #[must_use]
    pub fn help(&self) -> &'static str {
        match self {
            Self::InvalidPolicy => "Use a positive shutdown timeout representable by the system clock.",
            Self::Startup(_) => "Check executable permissions and process-containment support; do not set the private child environment marker yourself.",
            Self::Supervision(_) | Self::UnconfirmedTermination => "Preserve build outputs and investigate remaining worker processes before removing or rebuilding them; containers need a functioning child reaper (for example --init).",
            Self::Signal(_) => "Start bounded serving before installing another process-wide signal handler, and check that the operating system permits signal handler registration.",
            Self::Control(_) => "Keep the supervising parent alive and leave the worker's stdin reserved for shutdown control.",
            Self::Http(_) => "Check the HTTP error, then drop the watcher and join the active rebuild before returning.",
            Self::Forced(_) => "Inspect the active rebuild for hangs or increase the shutdown timeout; forced termination can leave incomplete outputs.",
        }
    }
}

#[cold]
#[inline(never)]
pub(super) fn protocol_error(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
