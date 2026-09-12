// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod consumer;
mod http;
mod workload;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use tokio::sync::mpsc;
use webui::streaming::{ChunkPool, StreamingWriter};

use consumer::Consumer;
use workload::Workload;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
type Pool = Option<Arc<ChunkPool>>;
const QUEUE_SLOTS: usize = 4;
const POOL_SLOTS: usize = 16;
const WARMUPS: usize = 5;

#[derive(Clone, serde::Serialize)]
struct Config {
    variant: String,
    hard: bool,
    chunk_bytes: usize,
    pool_bytes: usize,
    pooled: bool,
    workload: String,
    bytes_per_second: usize,
    samples: usize,
    iterations: usize,
}

impl Config {
    fn parse(args: &[String]) -> Result<Self> {
        if !(8..=9).contains(&args.len()) {
            return Err(
                "usage: --transport <account|measure|http-smoke|http-measure> \
                 <soft|hard|pool-only> <4096|16384|65536> <unpooled|pooled> \
                 <workload> <bytes/sec;0=unpaced> <samples> <iterations> [pool-bytes]"
                    .into(),
            );
        }
        let hard = match args[1].as_str() {
            "hard" => true,
            "soft" | "pool-only" => false,
            _ => return Err("variant must be soft, hard, or pool-only".into()),
        };
        let chunk_bytes = args[2].parse()?;
        if ![4096, 16384, 65536].contains(&chunk_bytes) {
            return Err("chunk size must be 4096, 16384, or 65536".into());
        }
        let pooled = match args[3].as_str() {
            "pooled" => true,
            "unpooled" => false,
            _ => return Err("pool mode must be pooled or unpooled".into()),
        };
        let config = Self {
            variant: args[1].clone(),
            hard,
            chunk_bytes,
            pool_bytes: match args.get(8) {
                Some(bytes) => bytes.parse()?,
                None => chunk_bytes,
            },
            pooled,
            workload: args[4].clone(),
            bytes_per_second: args[5].parse()?,
            samples: args[6].parse()?,
            iterations: args[7].parse()?,
        };
        if config.samples == 0 || config.iterations == 0 || config.pool_bytes == 0 {
            return Err("samples, iterations, and pool bytes must be positive".into());
        }
        Ok(config)
    }

    fn pool(&self) -> Pool {
        self.pooled
            .then(|| Arc::new(ChunkPool::new(POOL_SLOTS, self.pool_bytes)))
    }

    fn writer(&self, tx: mpsc::Sender<Bytes>, pool: &Pool) -> StreamingWriter {
        let writer = match pool {
            Some(pool) => StreamingWriter::new_pooled(tx, Arc::clone(pool)),
            None => StreamingWriter::new(tx),
        };
        // Preserve the true default construction path at 4 KiB, including the
        // original soft writer's headroom. Other sizes use the existing API.
        if self.chunk_bytes == StreamingWriter::CHUNK_TARGET {
            writer
        } else {
            writer.with_chunk_size(self.chunk_bytes)
        }
    }
}

pub(super) fn run(args: Vec<String>) -> Result<()> {
    if args.first().map(String::as_str) == Some("smoke") {
        if args.len() != 2 {
            return Err("usage: --transport smoke <soft|hard|pool-only>".into());
        }
        let hard = match args[1].as_str() {
            "hard" => true,
            "soft" | "pool-only" => false,
            _ => return Err("smoke variant must be soft, hard, or pool-only".into()),
        };
        return smoke(hard);
    }
    let config = Config::parse(&args)?;
    if matches!(args[0].as_str(), "measure" | "http-measure")
        && std::env::var("WEBUI_TRANSPORT_MEASURE").as_deref() != Ok("1")
    {
        return Err("timed runs require explicit WEBUI_TRANSPORT_MEASURE=1 approval".into());
    }
    let workload = Workload::new(&config.workload)?;
    match args[0].as_str() {
        "account" => account(&config, &workload),
        "measure" => measure(&config, &workload),
        "http-smoke" => http::run(config, workload, false),
        "http-measure" => http::run(config, workload, true),
        _ => Err("unknown mode; use smoke, account, measure, http-smoke, or http-measure".into()),
    }
}

fn smoke(hard: bool) -> Result<()> {
    for name in workload::NAMES {
        let workload = Workload::new(name)?;
        let reference = Arc::new(workload.reference()?);
        for chunk_bytes in [4096, 16384, 65536] {
            for pooled in [false, true] {
                let config = Config {
                    variant: if hard { "hard" } else { "soft" }.into(),
                    hard,
                    chunk_bytes,
                    pool_bytes: chunk_bytes,
                    pooled,
                    workload: (*name).into(),
                    bytes_per_second: 0,
                    samples: 1,
                    iterations: 1,
                };
                let pool = config.pool();
                let consumer = Consumer::new(Arc::clone(&reference), &config);
                let observation = consumer.execute(&workload, &config, &pool, false)?;
                consumer.finish()?;
                println!(
                    "smoke {name} {chunk_bytes} pooled={pooled} bytes={} chunks={} max={} sha256={}",
                    reference.bytes.len(), observation.chunks,
                    observation.max_chunk_bytes, reference.sha256()
                );
            }
        }
    }
    let args: Vec<String> = [
        "account",
        if hard { "hard" } else { "soft" },
        "4096",
        "pooled",
        "mixed",
        "8388608",
        "1",
        "1",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let config = Config::parse(&args)?;
    let workload = Workload::new(&config.workload)?;
    let consumer = Consumer::new(Arc::new(workload.reference()?), &config);
    consumer.execute(&workload, &config, &config.pool(), false)?;
    consumer.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_matches_pool_size_unless_explicitly_overridden() -> Result<()> {
        let mut args: Vec<String> = ["account", "soft", "4096", "pooled", "tiny", "0", "1", "1"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(Config::parse(&args)?.pool_bytes, 4096);
        args.push("5120".into());
        assert_eq!(Config::parse(&args)?.pool_bytes, 5120);
        args[2] = "0".into();
        assert!(Config::parse(&args).is_err());
        Ok(())
    }

    #[test]
    fn real_writer_preserves_content_and_flushes_at_all_sizes() -> Result<()> {
        // Do not assume which source snapshot the example is compiled against.
        // Hard maximum assertions are enabled by the variant-labelled smoke run.
        let workload = Workload::new("progressive")?;
        let reference = Arc::new(workload.reference()?);
        for chunk in ["4096", "16384", "65536"] {
            for pooled in ["pooled", "unpooled"] {
                let args = [
                    "account",
                    "soft",
                    chunk,
                    pooled,
                    "progressive",
                    "0",
                    "1",
                    "1",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
                let config = Config::parse(&args)?;
                let consumer = Consumer::new(Arc::clone(&reference), &config);
                let pool = config.pool();
                consumer.execute(&workload, &config, &pool, false)?;
                consumer.execute(&workload, &config, &pool, false)?;
                consumer.finish()?;
            }
        }
        Ok(())
    }
}

#[derive(serde::Serialize)]
struct Accounting<'a> {
    config: &'a Config,
    phase: &'static str,
    output_bytes: usize,
    output_sha256: String,
    flush_offsets: &'a [usize],
    chunks: usize,
    max_chunk_bytes: usize,
    allocations: usize,
    requested_bytes: usize,
    idle_buffers: usize,
    queue_slots: usize,
    pool_slots: usize,
    source_queue_payload_bound: Option<usize>,
    source_idle_capacity_bound: Option<usize>,
    process_rss_high_water_bytes: i64,
}

fn account(config: &Config, workload: &Workload) -> Result<()> {
    let reference = Arc::new(workload.reference()?);
    let consumer = Consumer::new(Arc::clone(&reference), config);
    let pool = config.pool();
    for phase in ["cold_pool", "warm_pool"] {
        if phase == "warm_pool" {
            for _ in 0..WARMUPS {
                consumer.execute(workload, config, &pool, false)?;
            }
        }
        crate::COUNT_ENABLED.store(true, Ordering::Relaxed);
        let before = crate::alloc_snapshot();
        let observation = consumer.execute(workload, config, &pool, false)?;
        let after = crate::alloc_snapshot();
        crate::COUNT_ENABLED.store(false, Ordering::Relaxed);
        let usage = crate::ProcessUsage::now();
        let idle_buffers = pool.as_ref().map_or(0, |pool| pool.idle_count());
        let row = Accounting {
            config,
            phase,
            output_bytes: reference.bytes.len(),
            output_sha256: reference.sha256(),
            flush_offsets: &reference.flushes,
            chunks: observation.chunks,
            max_chunk_bytes: observation.max_chunk_bytes,
            allocations: after.0 - before.0,
            requested_bytes: after.1 - before.1,
            idle_buffers,
            queue_slots: QUEUE_SLOTS,
            pool_slots: if config.pooled { POOL_SLOTS } else { 0 },
            source_queue_payload_bound: config.hard.then_some(QUEUE_SLOTS * config.chunk_bytes),
            source_idle_capacity_bound: if config.pooled {
                (config.hard || config.variant == "pool-only")
                    .then_some(POOL_SLOTS * config.pool_bytes)
            } else {
                Some(0)
            },
            process_rss_high_water_bytes: usage.max_rss_bytes(),
        };
        println!("{}", serde_json::to_string(&row)?);
    }
    consumer.finish()
}

#[derive(serde::Serialize)]
struct Sample<'a> {
    config: &'a Config,
    sample: usize,
    output_bytes: usize,
    wall_ns: u128,
    user_cpu_ns: u128,
    system_cpu_ns: u128,
    requests: Vec<consumer::Observation>,
}

fn measure(config: &Config, workload: &Workload) -> Result<()> {
    let reference = Arc::new(workload.reference()?);
    let consumer = Consumer::new(Arc::clone(&reference), config);
    let pool = config.pool();
    for _ in 0..WARMUPS {
        consumer.execute(workload, config, &pool, false)?;
    }
    for sample in 0..config.samples {
        let mut requests = Vec::with_capacity(config.iterations);
        let before = crate::ProcessUsage::now();
        let start = Instant::now();
        for _ in 0..config.iterations {
            requests.push(consumer.execute(workload, config, &pool, true)?);
        }
        let wall_ns = start.elapsed().as_nanos();
        let after = crate::ProcessUsage::now();
        let row = Sample {
            config,
            sample,
            output_bytes: reference.bytes.len(),
            wall_ns,
            user_cpu_ns: (after.user_cpu - before.user_cpu).as_nanos(),
            system_cpu_ns: (after.sys_cpu - before.sys_cpu).as_nanos(),
            requests,
        };
        println!("{}", serde_json::to_string(&row)?);
    }
    consumer.finish()
}
