// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Example-only [`FlushWriter`] adapter that paces boundary flushes.
//!
//! `render_streaming` flushes synchronously at each committed boundary
//! checkpoint. Left alone, a fast in-process render commits all of this
//! example's boundaries back-to-back, which would make the network timing
//! between chunks nondeterministic and too fast for Playwright to observe
//! reliably. `CheckpointPacedWriter` wraps any [`FlushWriter`] and sleeps
//! *after* delegating each flush to the wrapped writer, so:
//!
//! - Backpressure is unaffected: the delay never touches the wrapped
//!   writer's internal buffering/backpressure, it only runs after the
//!   flush has already happened.
//! - Disconnect/timeout propagation is unaffected: the wrapped writer's
//!   error is returned immediately, before the flush counter advances or
//!   any sleep happens, so a client disconnect never triggers a wasted
//!   sleep.
//! - No envelope, chunk, or boundary payload is manufactured or split by
//!   this adapter — it only observes flush *calls* the handler already
//!   makes and defers returning from them.
//!
//! The delay is supplied per flush rather than as one fixed duration,
//! because this app's boundaries do not all deserve the same gap: the
//! weather shell must hand straight over to the composer, while each feed
//! batch should arrive after a visible, randomized pause.

use std::time::Duration;

use webui_handler::{FlushWriter, ResponseWriter, Result};

/// Wraps a [`FlushWriter`] and sleeps for a caller-chosen duration after
/// each flush.
pub(crate) struct CheckpointPacedWriter<W, D> {
    inner: W,
    delay_for_flush: D,
    flushes_seen: usize,
}

impl<W, D: FnMut(usize) -> Duration> CheckpointPacedWriter<W, D> {
    /// `delay_for_flush` receives the zero-based index of the flush that
    /// just completed and returns how long to pause before returning from
    /// it. A [`Duration::ZERO`] never sleeps.
    pub(crate) fn new(inner: W, delay_for_flush: D) -> Self {
        Self {
            inner,
            delay_for_flush,
            flushes_seen: 0,
        }
    }
}

impl<W: ResponseWriter, D> ResponseWriter for CheckpointPacedWriter<W, D> {
    fn write(&mut self, content: &str) -> Result<()> {
        self.inner.write(content)
    }

    fn end(&mut self) -> Result<()> {
        self.inner.end()
    }
}

impl<W: FlushWriter, D: FnMut(usize) -> Duration> FlushWriter for CheckpointPacedWriter<W, D> {
    fn flush(&mut self) -> Result<()> {
        // Propagate the wrapped writer's error (disconnect/timeout) before
        // touching the counter or sleeping — a client that has already
        // gone away must never cause this adapter to block the render
        // thread further.
        self.inner.flush()?;
        let index = self.flushes_seen;
        self.flushes_seen += 1;
        let delay = (self.delay_for_flush)(index);
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::CheckpointPacedWriter;
    use std::time::{Duration, Instant};
    use webui_handler::{FlushWriter, HandlerError, ResponseWriter, Result};

    #[derive(Default)]
    struct RecordingWriter {
        writes: Vec<String>,
        flushes: usize,
        ended: bool,
        fail_after: Option<usize>,
    }

    impl ResponseWriter for RecordingWriter {
        fn write(&mut self, content: &str) -> Result<()> {
            self.writes.push(content.to_string());
            Ok(())
        }

        fn end(&mut self) -> Result<()> {
            self.ended = true;
            Ok(())
        }
    }

    impl FlushWriter for RecordingWriter {
        fn flush(&mut self) -> Result<()> {
            self.flushes += 1;
            if self.fail_after == Some(self.flushes) {
                return Err(HandlerError::ClientDisconnected);
            }
            Ok(())
        }
    }

    #[test]
    fn each_flush_receives_its_own_index() {
        let mut seen = Vec::new();
        {
            let mut writer = CheckpointPacedWriter::new(RecordingWriter::default(), |index| {
                seen.push(index);
                Duration::ZERO
            });
            for _ in 0..4 {
                writer
                    .flush()
                    .unwrap_or_else(|e| panic!("flush failed: {e}"));
            }
        }
        assert_eq!(seen, vec![0, 1, 2, 3]);
    }

    #[test]
    fn sleeps_only_for_the_flushes_the_schedule_selects() {
        let delay = Duration::from_millis(40);
        let mut writer = CheckpointPacedWriter::new(RecordingWriter::default(), move |index| {
            if index == 1 {
                delay
            } else {
                Duration::ZERO
            }
        });

        let start = Instant::now();
        writer
            .flush()
            .unwrap_or_else(|e| panic!("first flush failed: {e}"));
        assert!(
            start.elapsed() < delay,
            "flush 0 is unscheduled and must not sleep, took {:?}",
            start.elapsed()
        );

        let start = Instant::now();
        writer
            .flush()
            .unwrap_or_else(|e| panic!("second flush failed: {e}"));
        assert!(
            start.elapsed() >= delay,
            "flush 1 is scheduled and must sleep, took {:?}",
            start.elapsed()
        );

        let start = Instant::now();
        writer
            .flush()
            .unwrap_or_else(|e| panic!("third flush failed: {e}"));
        assert!(
            start.elapsed() < delay,
            "flush 2 is unscheduled and must not sleep, took {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn disconnect_propagates_without_sleeping_or_consulting_the_schedule() {
        let inner = RecordingWriter {
            fail_after: Some(1),
            ..RecordingWriter::default()
        };
        let mut consulted = false;
        let start = Instant::now();
        let result = {
            let mut writer = CheckpointPacedWriter::new(inner, |_| {
                consulted = true;
                Duration::from_secs(5)
            });
            writer.flush()
        };
        assert!(result.is_err(), "expected the disconnect error to surface");
        assert!(
            !consulted,
            "a disconnected client must not even price the delay"
        );
        assert!(
            start.elapsed() < Duration::from_millis(200),
            "must not sleep after a disconnect, took {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn write_and_end_delegate_unchanged() {
        let mut writer = CheckpointPacedWriter::new(RecordingWriter::default(), |_| Duration::ZERO);
        writer
            .write("hello")
            .unwrap_or_else(|e| panic!("write failed: {e}"));
        writer.end().unwrap_or_else(|e| panic!("end failed: {e}"));
        assert_eq!(writer.inner.writes, vec!["hello".to_string()]);
        assert!(writer.inner.ended);
    }

    #[test]
    fn an_all_zero_schedule_never_sleeps() {
        let mut writer = CheckpointPacedWriter::new(RecordingWriter::default(), |_| Duration::ZERO);
        let start = Instant::now();
        for _ in 0..5 {
            writer
                .flush()
                .unwrap_or_else(|e| panic!("flush failed: {e}"));
        }
        assert!(start.elapsed() < Duration::from_millis(50));
    }
}
