// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Generic rebuild-worker driver for dev servers.
//!
//! Spawns a background OS thread that owns the rebuild loop:
//!  1. Block waiting for a tick.
//!  2. Drain any extra ticks queued before the thread woke up — they
//!     coalesce into a single rebuild.
//!  3. Run the user-supplied rebuild closure.
//!  4. Drain any ticks that arrived during the rebuild — if any did,
//!     re-tick ourselves so the next iteration runs without waiting.
//!  5. Report success / error via [`RebuildReporter`] and broadcast
//!     the corresponding live-reload event.
//!
//! The closure is the only thing that varies between consumers
//! (webui-cli builds and renders an app, webui-press rebuilds a docs
//! site). It returns `Result<Vec<String>, RebuildError>`: the `Ok` value is a
//! list of plain advisory lines to print under the rebuild line, and the `Err`
//! lets the worker print a terminal rendering and broadcast a plain one to
//! browsers.

use std::path::PathBuf;
use std::sync::mpsc::{sync_channel, SyncSender};
use std::thread;

use crate::livereload::LiveReload;
use crate::reporter::RebuildReporter;

/// Tick-channel capacity. Bounded so a watcher event burst can't
/// unboundedly queue inside the rebuild worker. The worker drains all
/// pending ticks before each build, so a dropped `try_send` on a full
/// channel coalesces the same way the drain loop does.
const TICK_CHANNEL_CAPACITY: usize = 8;

/// Tick sender given to the watcher closure. Cheap to clone.
///
/// Each tick carries the changed paths from one debounce window, so the worker
/// can name the file(s) that triggered the rebuild. Send an empty `Vec` to
/// force a rebuild with no attributed trigger.
pub type TickSender = SyncSender<Vec<PathBuf>>;

/// Failure returned by a rebuild closure.
///
/// Carries two renderings of the same error so the worker can route each to
/// the surface that needs it:
///
/// - `display` — text for the terminal [`RebuildReporter`]. May embed ANSI
///   color, but each line must keep its **own** self-contained SGR span:
///   a single span that straddles newlines bleeds when the line is later
///   re-prefixed (e.g. `[server]` under `xtask dev`).
/// - `message` — plain, color-free text broadcast to connected browsers over
///   live-reload (logged to the dev console / shown in overlays). Never embed
///   ANSI here — browsers render escape codes as literal garbage.
///
/// Use [`RebuildError::plain`] (or the `From<String>` / `From<&str>`
/// conversions) when one rendering serves both surfaces.
pub struct RebuildError {
    display: String,
    message: String,
}

impl RebuildError {
    /// Build an error with distinct terminal `display` and browser `message`
    /// renderings. The caller owns colorization of `display`.
    #[must_use]
    pub fn new(display: String, message: String) -> Self {
        Self { display, message }
    }

    /// Build an error whose terminal and browser renderings are identical.
    /// Use when the failure has no structured form to colorize.
    #[must_use]
    pub fn plain(text: String) -> Self {
        Self {
            display: text.clone(),
            message: text,
        }
    }
}

impl From<String> for RebuildError {
    fn from(text: String) -> Self {
        Self::plain(text)
    }
}

impl From<&str> for RebuildError {
    fn from(text: &str) -> Self {
        Self::plain(text.to_owned())
    }
}

/// Spawn the rebuild worker on a dedicated OS thread and return the
/// sender used to enqueue rebuild ticks. The watcher closure should
/// call `tx.try_send(paths)` with the changed paths for every filesystem
/// event burst — failed sends (channel full) are intentional coalescing.
///
/// The closure runs synchronously on the worker thread, so it may use
/// blocking I/O freely. It returns `Ok(warnings)` on success (a possibly-empty
/// list of plain, color-free advisory lines to print under the rebuild line) or
/// `Err(RebuildError)` on failure; the worker prints the error's terminal
/// rendering and broadcasts its plain message to connected browsers.
///
/// The returned [`TickSender`] does not need to be held to keep the
/// worker alive — the worker stops only when every clone of the
/// sender is dropped.
pub fn spawn_rebuild_worker<F>(livereload: LiveReload, mut rebuild: F) -> TickSender
where
    F: FnMut() -> Result<Vec<String>, RebuildError> + Send + 'static,
{
    let (tx, rx) = sync_channel::<Vec<PathBuf>>(TICK_CHANNEL_CAPACITY);
    thread::spawn(move || {
        let mut reporter = RebuildReporter::new();
        let mut dirty = false;
        // Paths from ticks that arrived during the previous build belong to the
        // rebuild we are about to run, so carry them across iterations.
        let mut carryover: Vec<PathBuf> = Vec::new();
        loop {
            let mut triggers = std::mem::take(&mut carryover);
            if dirty {
                // A rebuild we just finished was racing with new events.
                // Skip the blocking recv so we rebuild immediately —
                // but still drain any ticks that piled up so they
                // collapse into this iteration.
                while let Ok(paths) = rx.try_recv() {
                    triggers.extend(paths);
                }
            } else {
                // Block for the first tick. Channel closed (every
                // external sender dropped) → exit cleanly so the
                // process can shut down without a zombie thread.
                match rx.recv() {
                    Ok(paths) => triggers.extend(paths),
                    Err(_) => break,
                }
                // Drain extra ticks that piled up while we were
                // waiting — they all collapse into this rebuild.
                while let Ok(paths) = rx.try_recv() {
                    triggers.extend(paths);
                }
            }

            let start = std::time::Instant::now();
            let result = rebuild();

            // Drain ticks that arrived during the build (= dirty events).
            // If any showed up, the next iteration runs without blocking on
            // `recv` so the user sees their change ASAP; their paths carry over
            // so they are attributed to that next rebuild, not this one.
            dirty = false;
            while let Ok(paths) = rx.try_recv() {
                dirty = true;
                carryover.extend(paths);
            }

            match result {
                Ok(warnings) => {
                    reporter.success(start.elapsed(), &triggers);
                    reporter.warnings(&warnings);
                    livereload.broadcast_reload();
                }
                Err(e) => {
                    reporter.error(&e.display);
                    livereload.broadcast_error(e.message);
                }
            }
        }
    });
    tx
}
