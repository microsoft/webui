// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Debounced filesystem watcher.
//!
//! Wraps `notify-debouncer-mini` so callers don't need to deal with the
//! debouncer event type. Returns a [`WatcherHandle`] that owns the
//! background thread; **the handle must be kept alive** for the watcher
//! to run.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use notify::RecommendedWatcher;
use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, Debouncer};

/// Owns the watcher background thread. Drop to stop watching.
///
/// `notify-debouncer-mini` spawns its background thread inside the
/// debouncer struct; dropping the struct kills the thread. Consumers
/// should hold this handle for the lifetime of their server.
pub struct WatcherHandle {
    _debouncer: Debouncer<RecommendedWatcher>,
}

/// Configuration for [`spawn_watcher`].
pub struct WatchConfig {
    /// Roots to watch recursively. Non-existent entries are silently
    /// skipped.
    pub paths: Vec<PathBuf>,
    /// Subtrees to ignore. An event is suppressed when its path lives
    /// underneath any entry here. Typical values: the build's `out_dir`,
    /// `node_modules`, `.git`, `target`.
    ///
    /// Each entry is canonicalized at registration time so symlink and
    /// path-form differences (`./dist` vs `dist`) compare correctly.
    pub ignore: Vec<PathBuf>,
    /// Debounce window — events arriving within this window are
    /// coalesced into a single callback invocation.
    pub debounce: Duration,
}

/// Start a debounced recursive watcher.
///
/// The closure `on_event` is invoked once per debounce window with the
/// owned, deduplicated list of paths that changed outside any
/// `cfg.ignore` root. If every event in a window targets an ignored
/// subtree, the callback is not invoked.
///
/// Non-existent paths in `cfg.paths` are silently skipped; this matches
/// the dev-server use case where some watched directories (e.g. an
/// optional `public/`) may not exist yet.
///
/// # Errors
///
/// Returns an error if the watcher cannot be created or if a path that
/// exists cannot be watched (typically a permissions issue).
pub fn spawn_watcher<F>(cfg: WatchConfig, on_event: F) -> Result<WatcherHandle>
where
    F: Fn(Vec<PathBuf>) + Send + 'static,
{
    // Canonicalize ignore paths once, up front. Non-existent ignore
    // entries are kept as-is so they still match if the path appears
    // mid-session (e.g. a fresh `dist/` created by the first build).
    let ignore: Vec<PathBuf> = cfg
        .ignore
        .iter()
        .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
        .collect();

    let mut debouncer = new_debouncer(cfg.debounce, move |res: DebounceEventResult| match res {
        Ok(events) => {
            // Filter out ignored paths and dedupe (notify can emit
            // duplicate events for the same path within a window).
            let mut paths: Vec<PathBuf> = Vec::with_capacity(events.len());
            for e in events {
                if is_ignored(&e.path, &ignore) {
                    continue;
                }
                if !paths.contains(&e.path) {
                    paths.push(e.path);
                }
            }
            if !paths.is_empty() {
                on_event(paths);
            }
        }
        Err(e) => {
            eprintln!("watcher error: {e:?}");
        }
    })
    .context("Cannot start file watcher")?;

    for p in &cfg.paths {
        if p.exists() {
            debouncer
                .watcher()
                .watch(p, RecursiveMode::Recursive)
                .with_context(|| format!("Cannot watch {}", p.display()))?;
        }
    }

    Ok(WatcherHandle {
        _debouncer: debouncer,
    })
}

/// Returns true when `event_path` lives under any ignored root.
///
/// Compares against the canonicalized form of `event_path` when
/// available (handles `./dist` vs `dist` and symlinks); falls back to
/// the raw path when canonicalization fails (e.g. the file was just
/// deleted).
fn is_ignored(event_path: &Path, ignore: &[PathBuf]) -> bool {
    if ignore.is_empty() {
        return false;
    }
    let canon = std::fs::canonicalize(event_path).ok();
    let candidate: &Path = canon.as_deref().unwrap_or(event_path);
    ignore.iter().any(|root| candidate.starts_with(root))
}

/// Default ignore subtrees common to dev servers. Includes the universal
/// junk: `node_modules`, `.git`, `target`. Callers should append their
/// own build-output directory and any tool-specific cache directories
/// before passing this to [`spawn_watcher`].
#[must_use]
pub fn default_ignore_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("node_modules"),
        PathBuf::from(".git"),
        PathBuf::from("target"),
    ]
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn is_ignored_matches_descendant() {
        let dir = tempfile::tempdir().unwrap();
        let dist = dir.path().join("dist");
        std::fs::create_dir(&dist).unwrap();
        let nested = dist.join("a/b.html");
        std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
        std::fs::write(&nested, "x").unwrap();

        let ignore = vec![std::fs::canonicalize(&dist).unwrap()];
        assert!(is_ignored(&nested, &ignore));
    }

    #[test]
    fn is_ignored_misses_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let dist = dir.path().join("dist");
        let other = dir.path().join("src/a.md");
        std::fs::create_dir(&dist).unwrap();
        std::fs::create_dir_all(other.parent().unwrap()).unwrap();
        std::fs::write(&other, "x").unwrap();

        let ignore = vec![std::fs::canonicalize(&dist).unwrap()];
        assert!(!is_ignored(&other, &ignore));
    }

    #[test]
    fn is_ignored_is_false_when_no_ignores() {
        let p = Path::new("/anything/at/all");
        assert!(!is_ignored(p, &[]));
    }
}
