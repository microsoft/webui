// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Example-only [`FlushWriter`] adapter that paces boundary flushes.
//!
//! `render_streaming` flushes synchronously at each committed boundary
//! checkpoint. Left alone, a fast in-process render commits all of this
//! example's boundaries back-to-back, which would make the network timing
//! between chunks nondeterministic and too fast for Playwright to observe
//! reliably. `CheckpointPacedWriter` wraps any [`FlushWriter`] and sleeps
//! *after* delegating each of the first `paced_flushes` flushes to the
//! wrapped writer, so:
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

use std::time::Duration;

use webui_handler::{FlushWriter, ResponseWriter, Result};

/// Wraps a [`FlushWriter`] and sleeps after each of the first
/// `paced_flushes` calls to [`FlushWriter::flush`].
pub(crate) struct CheckpointPacedWriter<W> {
    inner: W,
    delay: Duration,
    flushes_seen: usize,
    paced_flushes: usize,
}

impl<W> CheckpointPacedWriter<W> {
    /// `delay` is applied after each of the first `paced_flushes` flushes;
    /// every later flush is immediate.
    pub(crate) fn new(inner: W, delay: Duration, paced_flushes: usize) -> Self {
        Self {
            inner,
            delay,
            flushes_seen: 0,
            paced_flushes,
        }
    }
}

impl<W: ResponseWriter> ResponseWriter for CheckpointPacedWriter<W> {
    fn write(&mut self, content: &str) -> Result<()> {
        self.inner.write(content)
    }

    fn end(&mut self) -> Result<()> {
        self.inner.end()
    }
}

impl<W: FlushWriter> FlushWriter for CheckpointPacedWriter<W> {
    fn flush(&mut self) -> Result<()> {
        // Propagate the wrapped writer's error (disconnect/timeout) before
        // touching the counter or sleeping — a client that has already
        // gone away must never cause this adapter to block the render
        // thread further.
        self.inner.flush()?;
        self.flushes_seen += 1;
        if self.flushes_seen <= self.paced_flushes && !self.delay.is_zero() {
            std::thread::sleep(self.delay);
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
    fn paces_only_the_configured_leading_flushes() {
        let delay = Duration::from_millis(30);
        let mut writer = CheckpointPacedWriter::new(RecordingWriter::default(), delay, 2);

        let start = Instant::now();
        writer
            .flush()
            .unwrap_or_else(|e| panic!("first flush failed: {e}"));
        writer
            .flush()
            .unwrap_or_else(|e| panic!("second flush failed: {e}"));
        let paced_elapsed = start.elapsed();
        assert!(
            paced_elapsed >= delay * 2,
            "expected at least two delays, got {paced_elapsed:?}"
        );

        let start = Instant::now();
        writer
            .flush()
            .unwrap_or_else(|e| panic!("third flush failed: {e}"));
        assert!(
            start.elapsed() < delay,
            "third flush should not be paced, took {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn disconnect_propagates_without_sleeping() {
        let delay = Duration::from_secs(5);
        let inner = RecordingWriter {
            fail_after: Some(1),
            ..RecordingWriter::default()
        };
        let mut writer = CheckpointPacedWriter::new(inner, delay, 3);

        let start = Instant::now();
        let result = writer.flush();
        assert!(result.is_err(), "expected the disconnect error to surface");
        assert!(
            start.elapsed() < Duration::from_millis(200),
            "must not sleep after a disconnect, took {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn write_and_end_delegate_unchanged() {
        let mut writer = CheckpointPacedWriter::new(RecordingWriter::default(), Duration::ZERO, 0);
        writer
            .write("hello")
            .unwrap_or_else(|e| panic!("write failed: {e}"));
        writer.end().unwrap_or_else(|e| panic!("end failed: {e}"));
        assert_eq!(writer.inner.writes, vec!["hello".to_string()]);
        assert!(writer.inner.ended);
    }

    #[test]
    fn unpaced_writer_never_sleeps() {
        let mut writer = CheckpointPacedWriter::new(RecordingWriter::default(), Duration::ZERO, 5);
        let start = Instant::now();
        for _ in 0..5 {
            writer
                .flush()
                .unwrap_or_else(|e| panic!("flush failed: {e}"));
        }
        assert!(start.elapsed() < Duration::from_millis(50));
    }
}
