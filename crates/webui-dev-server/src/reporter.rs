// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Terminal reporter for dev-server rebuild loops.
//!
//! [`RebuildReporter`] manages the rolling-line UX where consecutive
//! successful rebuilds repaint a single terminal line in place,
//! while errors and banners commit it with a newline. It also
//! emits a wall-clock timestamp so users can see when each
//! rebuild fired without needing to glance at a clock.
//!
//! Sample output across three rebuilds and one error:
//!
//! ```text
//! ⚡ WebUI dev server
//!   ➜ http://localhost:3333/
//!
//!   ↻ rebuilt in 0.3s 16:42:51   ← repainted in place
//!   ↻ rebuilt in 0.4s 16:42:58   ← (replaces previous line)
//!   ✘ build error: parse failed  ← commits previous line, prints below
//!   ↻ rebuilt in 0.2s 16:43:10
//! ```

use std::io::{IsTerminal, Write};
use std::time::Duration;

use console::style;

/// ANSI escape: carriage return + clear-to-end-of-line. Used to repaint
/// the previous "rebuilt" line in place when consecutive rebuilds fire.
const REWIND_LINE: &str = "\r\x1b[2K";

/// Stateful terminal reporter for rebuild loops.
///
/// Tracks whether the previous output was a rolling rebuild line so the
/// next line can either repaint it (consecutive success) or commit it
/// with a newline (error, banner). Cheap to construct; not `Clone` —
/// instances are typically owned by a single rebuild worker.
///
/// When stderr is not a TTY (piped to a logger, captured by tools like
/// `concurrently`, or running in CI), the rolling line is replaced with
/// plain newline-terminated lines so wrappers can flush each rebuild
/// promptly.
pub struct RebuildReporter {
    last_was_rebuild: bool,
    /// True when stderr is an interactive terminal that supports the
    /// `\r\x1b[2K` repaint trick. Captured once at construction —
    /// stream redirection rarely changes mid-process and re-querying
    /// per write would be wasteful.
    interactive: bool,
}

impl Default for RebuildReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl RebuildReporter {
    /// Create a fresh reporter. Detects whether stderr is a TTY once.
    #[must_use]
    pub fn new() -> Self {
        Self {
            last_was_rebuild: false,
            interactive: std::io::stderr().is_terminal(),
        }
    }

    /// Print a successful rebuild line. In an interactive terminal,
    /// consecutive successes repaint a single line in place; otherwise
    /// each rebuild prints its own newline-terminated line so log
    /// wrappers (`concurrently`, CI capture, file redirects) flush it.
    ///
    /// Always writes to **stderr** so it lands on the same stream as
    /// the rest of the dev server's diagnostic output (banners, field
    /// tables, errors).
    pub fn success(&mut self, elapsed: Duration) {
        let elapsed_str = format_elapsed(elapsed);
        if self.interactive {
            let prefix = if self.last_was_rebuild {
                REWIND_LINE
            } else {
                ""
            };
            eprint!(
                "{}  {} {} {} {}",
                prefix,
                style("↻").cyan().bold(),
                style("rebuilt").bold(),
                style(format!("in {elapsed_str}")).dim(),
                style(local_hms()).dim(),
            );
            let _ = std::io::stderr().flush();
        } else {
            eprintln!(
                "  {} {} {} {}",
                style("↻").cyan().bold(),
                style("rebuilt").bold(),
                style(format!("in {elapsed_str}")).dim(),
                style(local_hms()).dim(),
            );
        }
        self.last_was_rebuild = true;
    }

    /// Print a rebuild error, committing the previous rolling line first
    /// so it isn't overwritten.
    pub fn error(&mut self, msg: &str) {
        self.commit_pending();
        // Color only the leading marker here. The body (`msg`) arrives
        // pre-rendered from the caller — either plain, or per-line colorized
        // with self-contained SGR spans. We must never wrap it in one span
        // that straddles newlines: that bleeds when each line is later
        // re-emitted with a process prefix (e.g. `[server]`).
        eprintln!(
            "  {} {} {msg}",
            style("✘").red().bold(),
            style("build error:").red().bold(),
        );
    }

    /// Commit any pending rolling rebuild line with a trailing newline,
    /// so subsequent unrelated output (banner, info message) doesn't
    /// land on the same line. No-op when not in interactive mode
    /// (those lines are already newline-terminated).
    pub fn commit_pending(&mut self) {
        if self.last_was_rebuild && self.interactive {
            eprintln!();
        }
        self.last_was_rebuild = false;
    }
}

/// Format a rebuild duration with the smallest sensible unit:
/// sub-second elapses render as `123ms` (no fractional digit lost),
/// while longer elapses fall back to `1.2s` so the line stays compact.
fn format_elapsed(elapsed: Duration) -> String {
    let ms = elapsed.as_millis();
    if ms < 1_000 {
        format!("{ms}ms")
    } else {
        format!("{:.1}s", elapsed.as_secs_f32())
    }
}

/// Format wall-clock local time as `HH:MM:SS`. Falls back to UTC if the
/// platform refuses to expose the local offset (e.g. some sandboxed
/// environments where `time` cannot read the system timezone).
#[must_use]
pub fn local_hms() -> String {
    if let Ok(now) = time::OffsetDateTime::now_local() {
        return format!("{:02}:{:02}:{:02}", now.hour(), now.minute(), now.second());
    }
    let now = time::OffsetDateTime::now_utc();
    format!("{:02}:{:02}:{:02}", now.hour(), now.minute(), now.second())
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn local_hms_returns_eight_chars() {
        let s = local_hms();
        assert_eq!(s.len(), 8);
        assert_eq!(s.as_bytes()[2], b':');
        assert_eq!(s.as_bytes()[5], b':');
    }

    #[test]
    fn reporter_tracks_pending_state() {
        let mut r = RebuildReporter::new();
        assert!(!r.last_was_rebuild);
        r.success(Duration::from_millis(123));
        assert!(r.last_was_rebuild);
        r.commit_pending();
        assert!(!r.last_was_rebuild);
    }

    #[test]
    fn error_resets_pending() {
        let mut r = RebuildReporter::new();
        r.success(Duration::from_millis(50));
        r.error("oops");
        assert!(!r.last_was_rebuild);
    }

    #[test]
    fn format_elapsed_uses_ms_below_one_second() {
        assert_eq!(format_elapsed(Duration::from_millis(0)), "0ms");
        assert_eq!(format_elapsed(Duration::from_millis(7)), "7ms");
        assert_eq!(format_elapsed(Duration::from_millis(999)), "999ms");
    }

    #[test]
    fn format_elapsed_uses_seconds_at_or_above_one_second() {
        assert_eq!(format_elapsed(Duration::from_millis(1_000)), "1.0s");
        assert_eq!(format_elapsed(Duration::from_millis(1_250)), "1.2s");
        assert_eq!(format_elapsed(Duration::from_secs(12)), "12.0s");
    }
}
