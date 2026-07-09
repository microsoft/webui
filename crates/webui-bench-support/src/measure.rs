// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Measurement core: run a workload, capture the four dimensions, derive
//! per-iteration metrics.
//!
//! [`measure`] runs a thunk a fixed number of times and returns a raw
//! [`Measurement`] (wall time + CPU delta + allocation delta + peak RSS).
//! [`calibrate`] sizes the iteration count so even a sub-microsecond workload
//! runs long enough to clear the OS CPU-clock tick (~15.6 ms on Windows), and
//! [`bench`] combines the two. [`Measurement::per_iter`] turns a raw reading
//! into a [`PerIter`] carrying cpu (µs + CPU%), memory (allocs + bytes),
//! throughput (ops/s + MiB/s) and latency (wall µs). [`percentile`] serves
//! latency-distribution benches.

use crate::alloc;
use crate::cpu::ProcessUsage;
use std::time::{Duration, Instant};

/// Raw totals captured across `iters` runs of a workload.
#[derive(Copy, Clone, Debug)]
pub struct Measurement {
    /// Number of workload iterations measured.
    pub iters: usize,
    /// Wall-clock time for all `iters`.
    pub wall: Duration,
    /// User CPU time consumed across all `iters`.
    pub user_cpu: Duration,
    /// System CPU time consumed across all `iters`.
    pub sys_cpu: Duration,
    /// Allocations performed across all `iters`.
    pub allocs: usize,
    /// Bytes allocated across all `iters`.
    pub bytes: usize,
    /// Process peak RSS observed at the end of the run (`-1` if unavailable).
    pub max_rss_bytes: i64,
    /// Input bytes processed *per iteration* (the throughput axis); `0` when a
    /// bench has no meaningful input size.
    pub work_bytes: usize,
}

/// Per-iteration derived metrics covering all four resource dimensions.
#[derive(Copy, Clone, Debug)]
pub struct PerIter {
    /// Allocations per iteration.
    pub allocs: f64,
    /// Bytes allocated per iteration.
    pub bytes: f64,
    /// User CPU microseconds per iteration.
    pub user_us: f64,
    /// System CPU microseconds per iteration.
    pub sys_us: f64,
    /// Wall-clock microseconds per iteration (latency).
    pub wall_us: f64,
    /// `(user + sys) / wall * 100` — how CPU-bound the workload is.
    pub cpu_pct: f64,
    /// Single-thread throughput in operations per second (`1e6 / wall_us`).
    pub ops_per_s: f64,
    /// Input throughput in MiB/s (from `work_bytes`); `0.0` when unknown.
    pub work_mib_s: f64,
    /// Process peak RSS in bytes (`-1` if unavailable).
    pub max_rss_bytes: i64,
}

impl PerIter {
    /// Project this per-iteration CPU cost to "% of one core at `rps`
    /// requests/second" — the load-test framing.
    ///
    /// `(user_us + sys_us) * rps / 10_000`. At the WebUI load test's 150 RPS a
    /// value near `60.0` reproduces the reported ~60 % CPU.
    #[must_use]
    pub fn core_pct_at(&self, rps: f64) -> f64 {
        (self.user_us + self.sys_us) * rps / 10_000.0
    }
}

impl Measurement {
    /// Derive per-iteration metrics. Returns all-zero when `iters == 0`.
    #[must_use]
    pub fn per_iter(&self) -> PerIter {
        let n = self.iters as f64;
        if n <= 0.0 {
            return PerIter {
                allocs: 0.0,
                bytes: 0.0,
                user_us: 0.0,
                sys_us: 0.0,
                wall_us: 0.0,
                cpu_pct: 0.0,
                ops_per_s: 0.0,
                work_mib_s: 0.0,
                max_rss_bytes: self.max_rss_bytes,
            };
        }
        let user_us = self.user_cpu.as_secs_f64() * 1_000_000.0 / n;
        let sys_us = self.sys_cpu.as_secs_f64() * 1_000_000.0 / n;
        let wall_us = self.wall.as_secs_f64() * 1_000_000.0 / n;
        let cpu_pct = if wall_us > 0.0 {
            (user_us + sys_us) / wall_us * 100.0
        } else {
            0.0
        };
        let ops_per_s = if wall_us > 0.0 {
            1_000_000.0 / wall_us
        } else {
            0.0
        };
        let wall_s = self.wall.as_secs_f64() / n;
        let work_mib_s = if wall_s > 0.0 && self.work_bytes > 0 {
            (self.work_bytes as f64 / (1024.0 * 1024.0)) / wall_s
        } else {
            0.0
        };
        PerIter {
            allocs: self.allocs as f64 / n,
            bytes: self.bytes as f64 / n,
            user_us,
            sys_us,
            wall_us,
            cpu_pct,
            ops_per_s,
            work_mib_s,
            max_rss_bytes: self.max_rss_bytes,
        }
    }
}

const WARMUP_ITERS: usize = 3;

/// Convert a **pre-validated, non-negative, finite** `f64` to `usize`,
/// saturating at the ends. Callers must clamp the value first; this only exists
/// to make the final narrowing cast explicit and lint-clean.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn usize_from_f64(v: f64) -> usize {
    if v <= 0.0 {
        0
    } else if v >= usize::MAX as f64 {
        usize::MAX
    } else {
        // Non-negative and below usize::MAX: the cast neither wraps, loses a
        // sign, nor truncates a meaningful magnitude.
        v as usize
    }
}

/// Run `f` exactly `iters` times and capture the four dimensions.
///
/// `work_bytes` is the input size processed per iteration (for MiB/s
/// throughput); pass `0` when the bench has no meaningful input size. The
/// workload is warmed up a few times first so lazy initialisation (formatter
/// caches, allocator slabs) does not land inside the measured window.
pub fn measure<F>(iters: usize, work_bytes: usize, mut f: F) -> Measurement
where
    F: FnMut(),
{
    for _ in 0..WARMUP_ITERS {
        f();
    }

    let a0 = alloc::snapshot();
    let r0 = ProcessUsage::now();
    let t0 = Instant::now();

    for _ in 0..iters {
        f();
    }

    let wall = t0.elapsed();
    let r1 = ProcessUsage::now();
    let a1 = alloc::snapshot();

    let cpu = r1.since(r0);
    let allocs = a1.since(a0);
    Measurement {
        iters,
        wall,
        user_cpu: cpu.user_cpu,
        sys_cpu: cpu.sys_cpu,
        allocs: allocs.count,
        bytes: allocs.bytes,
        max_rss_bytes: r1.max_rss_bytes,
        work_bytes,
    }
}

/// Pick an iteration count so `f` runs for roughly `target` wall time.
///
/// Cheap workloads need thousands of iterations to exceed the OS CPU-clock
/// resolution; expensive ones need only a few dozen. The per-iteration cost is
/// probed once and the run sized to the time budget, clamped to `[min, max]`.
pub fn calibrate<F>(target: Duration, min: usize, max: usize, f: &mut F) -> usize
where
    F: FnMut(),
{
    for _ in 0..WARMUP_ITERS {
        f();
    }
    let probe = 16usize;
    let t0 = Instant::now();
    for _ in 0..probe {
        f();
    }
    let per = t0.elapsed().as_secs_f64() / probe as f64;
    if per <= 0.0 {
        return max;
    }
    let want = target.as_secs_f64() / per;
    let want = if want.is_finite() && want >= 1.0 {
        usize_from_f64(want)
    } else {
        min
    };
    want.clamp(min, max)
}

/// Calibrate the iteration count to `target`, then [`measure`] the workload.
pub fn bench<F>(
    target: Duration,
    work_bytes: usize,
    min: usize,
    max: usize,
    mut f: F,
) -> Measurement
where
    F: FnMut(),
{
    let iters = calibrate(target, min, max, &mut f);
    measure(iters, work_bytes, f)
}

/// The `p`th percentile (0–100) of `samples`, sorting in place. Returns `0` for
/// an empty slice. Nearest-rank on a zero-based index.
#[must_use]
pub fn percentile(samples: &mut [u128], p: f64) -> u128 {
    if samples.is_empty() {
        return 0;
    }
    samples.sort_unstable();
    let clamped = p.clamp(0.0, 100.0);
    let idx = usize_from_f64(((clamped / 100.0) * (samples.len() - 1) as f64).round());
    samples[idx.min(samples.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_iter_zero_iters_is_safe() {
        let m = Measurement {
            iters: 0,
            wall: Duration::ZERO,
            user_cpu: Duration::ZERO,
            sys_cpu: Duration::ZERO,
            allocs: 0,
            bytes: 0,
            max_rss_bytes: -1,
            work_bytes: 0,
        };
        let pi = m.per_iter();
        assert_eq!(pi.ops_per_s, 0.0);
        assert_eq!(pi.cpu_pct, 0.0);
    }

    #[test]
    fn per_iter_derives_ops_and_throughput() {
        // 100 iters, 1 s wall → 10 000 µs/op → 100 ops/s.
        let m = Measurement {
            iters: 100,
            wall: Duration::from_secs(1),
            user_cpu: Duration::from_millis(800),
            sys_cpu: Duration::from_millis(200),
            allocs: 500,
            bytes: 1024 * 100,
            max_rss_bytes: 4096,
            work_bytes: 1024 * 1024, // 1 MiB per op
        };
        let pi = m.per_iter();
        assert!((pi.wall_us - 10_000.0).abs() < 1.0);
        assert!((pi.ops_per_s - 100.0).abs() < 0.01);
        // (800ms + 200ms) / 1s = 100% CPU-bound.
        assert!((pi.cpu_pct - 100.0).abs() < 0.01);
        // 1 MiB per op at 10 ms/op → 100 MiB/s.
        assert!((pi.work_mib_s - 100.0).abs() < 0.5);
        assert_eq!(pi.allocs, 5.0);
    }

    #[test]
    fn core_pct_reproduces_load_test_framing() {
        let m = Measurement {
            iters: 1,
            wall: Duration::from_micros(4000),
            user_cpu: Duration::from_micros(3600),
            sys_cpu: Duration::from_micros(400),
            allocs: 0,
            bytes: 0,
            max_rss_bytes: -1,
            work_bytes: 0,
        };
        // 4000 µs/op total CPU × 150 RPS / 10_000 = 60 % of a core.
        assert!((m.per_iter().core_pct_at(150.0) - 60.0).abs() < 0.01);
    }

    #[test]
    fn percentile_nearest_rank() {
        let mut xs = [10u128, 20, 30, 40, 50];
        assert_eq!(percentile(&mut xs, 0.0), 10);
        assert_eq!(percentile(&mut xs, 50.0), 30);
        assert_eq!(percentile(&mut xs, 100.0), 50);
        assert_eq!(percentile(&mut [], 99.0), 0);
    }

    #[test]
    fn measure_runs_and_counts_iters() {
        let mut n = 0u64;
        let m = measure(1000, 0, || {
            n = n.wrapping_add(1);
        });
        assert_eq!(m.iters, 1000);
        // 1000 measured + warmups.
        assert!(n >= 1000);
    }

    #[test]
    fn calibrate_clamps_to_bounds() {
        // A no-op probes as ~0 cost → clamps up to max.
        let iters = calibrate(Duration::from_millis(10), 50, 5000, &mut || {});
        assert!((50..=5000).contains(&iters));
    }
}
