// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::sync::{mpsc as control, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::sync::mpsc;

use super::workload::{Reference, Workload};
use super::{Config, Result};

#[derive(Default, serde::Serialize)]
pub(super) struct Observation {
    pub(super) first_body_ns: u128,
    pub(super) drained_ns: u128,
    pub(super) completed_ns: u128,
    pub(super) producer_ns: u128,
    #[serde(skip)]
    pub(super) chunks: usize,
    #[serde(skip)]
    pub(super) max_chunk_bytes: usize,
}

struct Command {
    start: Option<Instant>,
    verify: bool,
}

pub(super) struct Consumer {
    tx: mpsc::Sender<Bytes>,
    commands: control::SyncSender<Command>,
    done: control::Receiver<Observation>,
    thread: JoinHandle<()>,
}

impl Consumer {
    pub(super) fn new(reference: Arc<Reference>, config: &Config) -> Self {
        let (tx, mut rx) = mpsc::channel::<Bytes>(super::QUEUE_SLOTS);
        let (commands, commands_rx) = control::sync_channel::<Command>(1);
        let (done_tx, done) = control::sync_channel(1);
        let rate = config.bytes_per_second;
        let hard_limit = config.hard.then_some(config.chunk_bytes);
        let thread = thread::spawn(move || {
            while let Ok(command) = commands_rx.recv() {
                let mut seen = 0;
                let mut checkpoint = 0;
                let mut observation = Observation::default();
                let mut pace_start = None;
                while seen < reference.bytes.len() {
                    let Some(chunk) = rx.blocking_recv() else {
                        panic!("producer ended before expected output");
                    };
                    assert!(!chunk.is_empty());
                    if seen == 0 {
                        if let Some(start) = command.start {
                            observation.first_body_ns = start.elapsed().as_nanos();
                        }
                        if rate != 0 {
                            pace_start = Some(Instant::now());
                        }
                    }
                    let end = seen + chunk.len();
                    if command.verify {
                        verify_chunk(&reference, &chunk, seen, &mut checkpoint, hard_limit);
                        observation.chunks += 1;
                        observation.max_chunk_bytes = observation.max_chunk_bytes.max(chunk.len());
                    }
                    seen = end;
                    if let Some(start) = pace_start {
                        if let Some(wait) = (start + service_time(seen, rate))
                            .checked_duration_since(Instant::now())
                        {
                            thread::sleep(wait);
                        }
                    }
                    // Hold exactly one received Bytes while its byte budget is serviced.
                    drop(chunk);
                }
                assert_eq!(seen, reference.bytes.len());
                if command.verify {
                    assert_eq!(checkpoint, reference.flushes.len());
                }
                if let Some(start) = command.start {
                    observation.drained_ns = start.elapsed().as_nanos();
                }
                if done_tx.send(observation).is_err() {
                    return;
                }
            }
            assert!(rx.try_recv().is_err(), "unexpected trailing output");
        });
        Self {
            tx,
            commands,
            done,
            thread,
        }
    }

    pub(super) fn execute(
        &self,
        workload: &Workload,
        config: &Config,
        pool: &super::Pool,
        timed: bool,
    ) -> Result<Observation> {
        let start = timed.then(Instant::now);
        self.commands.send(Command {
            start,
            verify: !timed,
        })?;
        let mut writer = config.writer(self.tx.clone(), pool);
        workload.write(&mut writer)?;
        drop(writer);
        let producer_ns = start.map_or(0, |time| time.elapsed().as_nanos());
        let mut observation = self.done.recv()?;
        observation.producer_ns = producer_ns;
        observation.completed_ns = start.map_or(0, |time| time.elapsed().as_nanos());
        Ok(observation)
    }

    pub(super) fn finish(self) -> Result<()> {
        drop(self.commands);
        drop(self.tx);
        self.thread.join().map_err(|_| "consumer panicked".into())
    }
}

fn verify_chunk(
    reference: &Reference,
    chunk: &[u8],
    seen: usize,
    checkpoint: &mut usize,
    hard_limit: Option<usize>,
) {
    let end = seen + chunk.len();
    assert!(end <= reference.bytes.len(), "extra output");
    assert_eq!(chunk, &reference.bytes[seen..end], "output mismatch");
    if let Some(limit) = hard_limit {
        assert!(chunk.len() <= limit, "hard maximum exceeded");
    }
    if let Some(&flush) = reference.flushes.get(*checkpoint) {
        assert!(end <= flush, "chunk crossed a semantic flush");
        if end == flush {
            *checkpoint += 1;
        }
    }
}

pub(super) fn service_time(bytes: usize, bytes_per_second: usize) -> Duration {
    if bytes_per_second == 0 {
        return Duration::ZERO;
    }
    Duration::from_secs_f64(bytes as f64 / bytes_per_second as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_pacing_budget_is_independent_of_chunk_count() {
        let total = 1024 * 1024;
        for chunk in [4096, 16384, 65536, total] {
            let mut previous = Duration::ZERO;
            let mut serviced = Duration::ZERO;
            for bytes in (chunk..=total).step_by(chunk) {
                let deadline = service_time(bytes, 8 * 1024 * 1024);
                serviced += deadline - previous;
                previous = deadline;
            }
            assert_eq!(serviced, Duration::from_millis(125));
        }
    }

    #[test]
    #[should_panic(expected = "chunk crossed a semantic flush")]
    fn verification_rejects_merged_checkpoints() {
        let reference = Reference {
            bytes: b"abcd".to_vec(),
            flushes: vec![2, 4],
        };
        verify_chunk(&reference, b"abcd", 0, &mut 0, None);
    }
}
