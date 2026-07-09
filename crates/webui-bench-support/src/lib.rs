// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Shared measurement and reporting harness for WebUI's **resource
//! benchmarks** — the CPU/memory/throughput/latency harnesses that live as
//! `examples/*_bench.rs` binaries and drive the `xtask bench` command.
//!
//! # Why this crate exists
//!
//! WebUI already uses [Criterion](https://docs.rs/criterion) for statistical
//! wall-clock micro-benchmarks (`benches/*.rs`). Criterion is excellent at what
//! it does, but it deliberately measures only **wall-clock time**. The problem
//! we actually chase — the Node SSR host burning ~60 % CPU vs ~25 % for a
//! JS control at 150 RPS — is a **CPU-time** and **allocation** problem, and
//! Criterion cannot see either of those. So the resource benches roll their own
//! measurement, and until now each one *re-implemented* the same primitives:
//!
//! * a `#[global_allocator]` that counts allocations and bytes,
//! * a cross-platform CPU-time reader (`getrusage` / `GetProcessTimes`),
//! * an iteration calibrator that clears the OS CPU-clock tick,
//! * a JSON baseline snapshot with `--save` / `--compare` regression gating,
//! * and a console-styled results table.
//!
//! Three ~800-line benches carried three drifting copies of all of the above.
//! This crate is the single, unit-tested home for those primitives so every
//! resource bench reports the **same four dimensions the same way**:
//!
//! | dimension    | primitive                                             |
//! |--------------|-------------------------------------------------------|
//! | **cpu**      | [`cpu::ProcessUsage`] (user + system µs, peak RSS)    |
//! | **memory**   | [`alloc::CountingAllocator`] (exact allocs + bytes)   |
//! | **latency**  | [`measure::Measurement`] wall time + [`measure::percentile`] |
//! | **throughput** | [`measure::PerIter`] ops/s + MiB/s                  |
//!
//! Regression gating is provided generically by [`baseline`], and consistent
//! tables by [`report`].
//!
//! # Layering
//!
//! ```text
//! Criterion            → statistical wall-clock micro-benchmarks (benches/*.rs)
//! webui-bench-support  → CPU/mem/throughput/latency resource benches (examples/*_bench.rs)
//! ```
//!
//! Both layers are complementary; this crate is **dev-only** (a `dev-dependency`
//! of the benched crate) and must never be pulled into a shipping build.

// The measurement primitives require `unsafe` to talk to the platform resource
// APIs (`GlobalAlloc`, `getrusage`, `GetProcessTimes`, `GetProcessMemoryInfo`).
// Each `unsafe` block upholds its contract with a local `// SAFETY:` note. The
// workspace-wide `unsafe_code = "deny"` lint targets shipping library code; this
// dev-only benchmarking crate is exempted at the crate level, exactly as the
// bench example binaries were before the extraction.
#![allow(unsafe_code)]

pub mod alloc;
pub mod baseline;
pub mod cpu;
pub mod measure;
pub mod report;

pub use alloc::{AllocStats, CountingAllocator};
pub use baseline::{BaselineRow, Metric};
pub use cpu::ProcessUsage;
pub use measure::{bench, calibrate, measure, percentile, Measurement, PerIter};
pub use report::Table;
