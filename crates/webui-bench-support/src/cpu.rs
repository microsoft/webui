// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Cross-platform process CPU-time reader — the **cpu** dimension.
//!
//! This is the primitive Criterion does not provide. It reads the process's
//! accumulated **user** and **system** CPU time (plus the peak resident set
//! size, a cheap by-product of the same syscalls) so a benchmark can report how
//! CPU-bound a stage is, not merely how long it took on the wall clock.
//!
//! * Unix — `getrusage(RUSAGE_SELF)` (`ru_utime`, `ru_stime`, `ru_maxrss`).
//! * Windows — `GetProcessTimes` (user/kernel `FILETIME`) and
//!   `GetProcessMemoryInfo` (`PeakWorkingSetSize`).
//!
//! On unsupported targets every field reads as zero / `-1` so callers still
//! compile and run (they just cannot attribute CPU).

use std::time::Duration;

/// A snapshot of the current process's accumulated CPU time and peak RSS.
///
/// Subtract two readings taken around a workload to get the CPU the workload
/// consumed; see [`ProcessUsage::since`].
#[derive(Copy, Clone, Debug)]
pub struct ProcessUsage {
    /// User-space CPU time accumulated by the process so far.
    pub user_cpu: Duration,
    /// Kernel/system CPU time accumulated by the process so far.
    pub sys_cpu: Duration,
    /// Peak resident set size in bytes, or `-1` when unavailable.
    ///
    /// This is a process-global high-water mark (monotonic), not a per-call
    /// delta — treat it as "largest the process ever got", not "this workload".
    pub max_rss_bytes: i64,
}

impl ProcessUsage {
    /// Read the current process CPU usage.
    #[must_use]
    pub fn now() -> Self {
        read_now()
    }

    /// The CPU consumed between an `earlier` reading and this one.
    ///
    /// `user_cpu` / `sys_cpu` are saturating deltas; `max_rss_bytes` carries the
    /// later (larger) high-water mark through unchanged.
    #[must_use]
    pub fn since(self, earlier: ProcessUsage) -> ProcessUsage {
        ProcessUsage {
            user_cpu: self.user_cpu.saturating_sub(earlier.user_cpu),
            sys_cpu: self.sys_cpu.saturating_sub(earlier.sys_cpu),
            max_rss_bytes: self.max_rss_bytes,
        }
    }

    /// Total CPU time (`user + sys`).
    #[must_use]
    pub fn total_cpu(&self) -> Duration {
        self.user_cpu.saturating_add(self.sys_cpu)
    }
}

#[cfg(unix)]
fn read_now() -> ProcessUsage {
    // SAFETY: `getrusage` writes into a fully-owned, zero-initialised `rusage`;
    // the struct is valid for the duration of the call and read only after it
    // succeeds.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    assert_eq!(rc, 0, "getrusage(RUSAGE_SELF) failed");
    ProcessUsage {
        user_cpu: timeval_to_duration(usage.ru_utime),
        sys_cpu: timeval_to_duration(usage.ru_stime),
        max_rss_bytes: unix_rss_to_bytes(usage.ru_maxrss),
    }
}

#[cfg(unix)]
fn timeval_to_duration(tv: libc::timeval) -> Duration {
    // CPU timevals are non-negative; `tv_usec` is always below 1_000_000.
    let secs = u64::try_from(tv.tv_sec).unwrap_or(0);
    let usecs = u32::try_from(tv.tv_usec).unwrap_or(0) % 1_000_000;
    Duration::new(secs, usecs * 1_000)
}

#[cfg(unix)]
#[allow(clippy::unnecessary_cast)]
fn unix_rss_to_bytes(raw: libc::c_long) -> i64 {
    // `c_long` is `i64` on LP64 targets (the cast is a no-op) and `i32` on
    // ILP32 targets (the cast widens); either way the value keeps its sign.
    let raw = raw as i64;
    // Linux/BSD report `ru_maxrss` in kilobytes; macOS reports bytes.
    if cfg!(target_os = "macos") {
        raw
    } else {
        raw.saturating_mul(1024)
    }
}

#[cfg(windows)]
fn read_now() -> ProcessUsage {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    // SAFETY: pseudo-handle from GetCurrentProcess is always valid.
    let process = unsafe { GetCurrentProcess() };

    let mut creation_time = FILETIME::default();
    let mut exit_time = FILETIME::default();
    let mut kernel_time = FILETIME::default();
    let mut user_time = FILETIME::default();
    // SAFETY: all four FILETIME pointers refer to writable, initialised locals;
    // `process` is the current-process pseudo-handle.
    let times_ok = unsafe {
        GetProcessTimes(
            process,
            &mut creation_time,
            &mut exit_time,
            &mut kernel_time,
            &mut user_time,
        )
    };
    assert_ne!(times_ok, 0, "GetProcessTimes failed");

    let counters_size = process_memory_counters_size();
    // SAFETY: a zeroed PROCESS_MEMORY_COUNTERS becomes valid once `cb` is set to
    // its own size, which the next line does before the call reads it.
    let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    counters.cb = counters_size;
    // SAFETY: `counters` is writable with `cb` initialised; `process` is valid.
    let memory_ok = unsafe { K32GetProcessMemoryInfo(process, &mut counters, counters_size) };
    assert_ne!(memory_ok, 0, "GetProcessMemoryInfo failed");

    ProcessUsage {
        user_cpu: filetime_to_duration(user_time),
        sys_cpu: filetime_to_duration(kernel_time),
        max_rss_bytes: usize_to_i64_saturating(counters.PeakWorkingSetSize),
    }
}

#[cfg(windows)]
fn filetime_to_duration(filetime: windows_sys::Win32::Foundation::FILETIME) -> Duration {
    let ticks = (u64::from(filetime.dwHighDateTime) << 32) | u64::from(filetime.dwLowDateTime);
    let secs = ticks / 10_000_000;
    // 100 ns per tick; the sub-second remainder always fits in u32 nanoseconds.
    let nanos = u32::try_from((ticks % 10_000_000) * 100).unwrap_or(0);
    Duration::new(secs, nanos)
}

#[cfg(windows)]
fn usize_to_i64_saturating(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[cfg(windows)]
fn process_memory_counters_size() -> u32 {
    u32::try_from(std::mem::size_of::<
        windows_sys::Win32::System::ProcessStatus::PROCESS_MEMORY_COUNTERS,
    >())
    .unwrap_or(0)
}

#[cfg(not(any(unix, windows)))]
fn read_now() -> ProcessUsage {
    ProcessUsage {
        user_cpu: Duration::ZERO,
        sys_cpu: Duration::ZERO,
        max_rss_bytes: -1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_time_is_monotonic() {
        let a = ProcessUsage::now();
        // Burn a little CPU so the reading advances on supported platforms.
        let mut acc = 0u64;
        for i in 0..2_000_000u64 {
            acc = acc.wrapping_add(i);
        }
        assert!(acc > 0);
        let b = ProcessUsage::now();
        assert!(b.total_cpu() >= a.total_cpu());
    }

    #[test]
    fn since_saturates_and_carries_rss() {
        let earlier = ProcessUsage {
            user_cpu: Duration::from_micros(100),
            sys_cpu: Duration::from_micros(50),
            max_rss_bytes: 1000,
        };
        let later = ProcessUsage {
            user_cpu: Duration::from_micros(250),
            sys_cpu: Duration::from_micros(70),
            max_rss_bytes: 2000,
        };
        let d = later.since(earlier);
        assert_eq!(d.user_cpu, Duration::from_micros(150));
        assert_eq!(d.sys_cpu, Duration::from_micros(20));
        assert_eq!(d.max_rss_bytes, 2000);
        // Reversed must not underflow.
        assert_eq!(earlier.since(later).user_cpu, Duration::ZERO);
    }
}
