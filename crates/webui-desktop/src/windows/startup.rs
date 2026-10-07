// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Opt-in UI-thread startup attribution, independent of benchmark readiness.
//!
//! Relative phase records retain their original format. Companion QPC records
//! carry the same phase, counter ticks, frequency and thread ID for ETW alignment.

use std::fmt;
use std::time::Instant;

use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::GetCurrentThreadId;

#[derive(Clone, Copy)]
pub(super) struct StartupTrace(Option<Instant>);

impl StartupTrace {
    pub(super) fn begin() -> Self {
        Self::new(std::env::var_os("WEBUI_WINDOWS_STARTUP_TRACE").is_some_and(|value| value == "1"))
    }

    fn new(enabled: bool) -> Self {
        Self(enabled.then(Instant::now))
    }

    pub(super) fn is_enabled(&self) -> bool {
        self.0.is_some()
    }

    pub(super) fn mark(&self, phase: &str) {
        if let Some(start) = self.0 {
            emit_mark(phase, start);
        }
    }
}

struct ClockReading {
    ticks: i64,
    frequency: i64,
    thread_id: u32,
}

impl fmt::Display for ClockReading {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} {} {}",
            self.ticks, self.frequency, self.thread_id
        )
    }
}

fn read_clock() -> windows::core::Result<ClockReading> {
    let mut frequency = 0;
    let mut ticks = 0;
    // SAFETY: Both queries write to live i64 values. GetCurrentThreadId has no
    // caller requirements and returns the thread that owns this phase marker.
    let thread_id = unsafe {
        QueryPerformanceFrequency(&mut frequency)?;
        QueryPerformanceCounter(&mut ticks)?;
        GetCurrentThreadId()
    };
    Ok(ClockReading {
        ticks,
        frequency,
        thread_id,
    })
}

#[cold]
#[inline(never)]
fn emit_mark(phase: &str, start: Instant) {
    let clock = read_clock();
    eprintln!(
        "WEBUI_WINDOWS_STARTUP {phase} {:.3}",
        start.elapsed().as_secs_f64() * 1000.0
    );
    match clock {
        Ok(clock) => eprintln!("WEBUI_WINDOWS_QPC {phase} {clock}"),
        Err(error) => eprintln!("WEBUI_WINDOWS_QPC_ERROR {phase} {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_tracing_is_opt_in_and_keeps_no_clock_when_disabled() {
        assert!(StartupTrace::new(false).0.is_none());
        assert!(!StartupTrace::new(false).is_enabled());
        let trace = StartupTrace::new(true);
        assert!(trace.0.is_some());
        assert!(trace.is_enabled());
        trace.mark("test");
    }

    #[test]
    fn startup_clock_records_have_stable_numeric_fields() {
        let clock = ClockReading {
            ticks: 123_456,
            frequency: 10_000_000,
            thread_id: 42,
        };
        assert_eq!(clock.to_string(), "123456 10000000 42");
    }

    #[test]
    fn startup_clock_uses_a_monotonic_counter_on_the_current_thread() -> windows::core::Result<()> {
        let first = read_clock()?;
        let second = read_clock()?;
        assert!(first.frequency > 0);
        assert_eq!(first.frequency, second.frequency);
        assert!(second.ticks >= first.ticks);
        assert_ne!(first.thread_id, 0);
        assert_eq!(first.thread_id, second.thread_id);
        Ok(())
    }
}
