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
//! site). It returns `Result<(), String>` so the worker can
//! uniformly format the error and broadcast it to connected browsers.

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
pub type TickSender = SyncSender<()>;

/// Spawn the rebuild worker on a dedicated OS thread and return the
/// sender used to enqueue rebuild ticks. The watcher closure should
/// call `tx.try_send(())` for every filesystem event burst — failed
/// sends (channel full) are intentional coalescing.
///
/// The closure runs synchronously on the worker thread, so it may use
/// blocking I/O freely. It returns `Ok(())` on success or `Err(msg)`
/// with a user-facing message on failure; the worker handles printing
/// and broadcasting based on the result.
///
/// The returned [`TickSender`] does not need to be held to keep the
/// worker alive — the worker stops only when every clone of the
/// sender is dropped.
pub fn spawn_rebuild_worker<F>(livereload: LiveReload, mut rebuild: F) -> TickSender
where
    F: FnMut() -> Result<(), String> + Send + 'static,
{
    let (tx, rx) = sync_channel::<()>(TICK_CHANNEL_CAPACITY);
    let self_tick = tx.clone();
    thread::spawn(move || {
        let mut reporter = RebuildReporter::new();
        loop {
            // Block for the first tick. Channel closed → exit.
            if rx.recv().is_err() {
                break;
            }
            // Drain extra ticks that piled up while we were waiting —
            // they all collapse into this single rebuild.
            while rx.try_recv().is_ok() {}

            let start = std::time::Instant::now();
            let result = rebuild();

            // Drain ticks that arrived during the build (= dirty events).
            // If any showed up, re-tick ourselves so the next iteration
            // rebuilds without waiting for new filesystem activity.
            let mut dirty = false;
            while rx.try_recv().is_ok() {
                dirty = true;
            }

            match result {
                Ok(()) => {
                    reporter.success(start.elapsed());
                    livereload.broadcast_reload();
                }
                Err(msg) => {
                    reporter.error(&msg);
                    livereload.broadcast_error(msg);
                }
            }

            if dirty {
                let _ = self_tick.try_send(());
            }
        }
    });
    tx
}
