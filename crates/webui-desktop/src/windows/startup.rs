// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Opt-in UI-thread startup attribution, independent of benchmark readiness.

use std::time::Instant;

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
            eprintln!(
                "WEBUI_WINDOWS_STARTUP {phase} {:.3}",
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
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
}
