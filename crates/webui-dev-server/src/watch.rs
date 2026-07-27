// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Debounced filesystem watcher.
//!
//! Wraps `notify-debouncer-mini` so callers don't need to deal with the
//! debouncer event type. Returns a [`WatcherHandle`] that owns the
//! background thread; **the handle must be kept alive** for the watcher
//! to run.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::Hasher;
use std::path::{Path, PathBuf};
use std::sync::Arc;
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
    /// Exact files to watch through their parent directories.
    ///
    /// These files bypass `ignore` filtering, which lets a manifest under an
    /// ignored output directory act as an explicit synchronization point
    /// without forwarding unrelated bundle writes.
    pub explicit_files: Vec<PathBuf>,
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
    /// Optional predicate that allows byte-identical file events through.
    ///
    /// `webui serve --watch` uses this while a rebuild error is active so a
    /// no-op save can retry transient failures. When the predicate returns
    /// `false` (the common clean state), identical saves are still dropped.
    pub retry_unchanged_when: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
}

/// Start a debounced recursive watcher.
///
/// The closure `on_event` is invoked once per debounce window with the
/// owned, deduplicated list of paths that changed outside any
/// `cfg.ignore` root. If every event in a window targets an ignored
/// subtree, the callback is not invoked.
///
/// Paths whose file content is byte-identical to the previous event are dropped
/// in the clean state, so a no-op save (e.g. repeated Ctrl+S that rewrites the
/// same bytes) triggers **no** rebuild. If `retry_unchanged_when` returns true,
/// unchanged events are forwarded so callers can retry an active error.
/// Deletions and files larger than an internal cap always count as changed.
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
    let explicit_files: Vec<PathBuf> = cfg
        .explicit_files
        .iter()
        .filter_map(|path| normalize_explicit_file(path))
        .collect();

    let mut content_hashes: HashMap<PathBuf, u64> = HashMap::new();
    let retry_unchanged_when = cfg.retry_unchanged_when.clone();
    let explicit_filter = explicit_files.clone();
    let mut debouncer = new_debouncer(cfg.debounce, move |res: DebounceEventResult| match res {
        Ok(events) => {
            // Filter out ignored paths and dedupe (notify can emit
            // duplicate events for the same path within a window).
            let mut paths: Vec<PathBuf> = Vec::with_capacity(events.len());
            let mut seen: HashSet<PathBuf> = HashSet::with_capacity(events.len());
            for e in events {
                if should_ignore_event(&e.path, &ignore, &explicit_filter) {
                    continue;
                }
                if seen.insert(e.path.clone()) {
                    paths.push(e.path);
                }
            }
            // Drop paths whose content is byte-identical to the last event.
            // Editors fire a write on every Ctrl+S even when nothing changed;
            // rebuilding then is pure wasted work, so a no-op save triggers no
            // rebuild at all in the clean state. When the caller reports that a
            // rebuild error is active, unchanged events are allowed through so a
            // no-op save can retry transient failures. Deletions and oversized
            // files always count as changed (see `content_changed`).
            let retry_unchanged = retry_unchanged_when
                .as_ref()
                .is_some_and(|predicate| predicate());
            paths.retain(|path| should_forward_path(&mut content_hashes, path, retry_unchanged));
            if !paths.is_empty() {
                on_event(paths);
            }
        }
        Err(e) => {
            eprintln!("watcher error: {e:?}");
        }
    })
    .context("Cannot start file watcher")?;

    let mut watched_roots = Vec::with_capacity(cfg.paths.len());
    for p in &cfg.paths {
        if p.exists() {
            watched_roots.push(std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()));
            debouncer
                .watcher()
                .watch(p, RecursiveMode::Recursive)
                .with_context(|| format!("Cannot watch {}", p.display()))?;
        }
    }
    let mut explicit_parents = HashSet::new();
    for file in &explicit_files {
        if watched_roots.iter().any(|root| file.starts_with(root)) {
            continue;
        }
        let Some(parent) = file.parent() else {
            continue;
        };
        if parent.exists() && explicit_parents.insert(parent.to_path_buf()) {
            debouncer
                .watcher()
                .watch(parent, RecursiveMode::NonRecursive)
                .with_context(|| format!("Cannot watch {}", parent.display()))?;
        }
    }

    Ok(WatcherHandle {
        _debouncer: debouncer,
    })
}

fn normalize_explicit_file(path: &Path) -> Option<PathBuf> {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return Some(canonical);
    }
    let parent = path
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = std::fs::canonicalize(parent).ok()?;
    Some(parent.join(path.file_name()?))
}

fn is_explicit_file(event_path: &Path, explicit_files: &[PathBuf]) -> bool {
    let normalized = std::fs::canonicalize(event_path).unwrap_or_else(|_| event_path.to_path_buf());
    explicit_files.iter().any(|path| path == &normalized)
}

fn should_ignore_event(event_path: &Path, ignore: &[PathBuf], explicit_files: &[PathBuf]) -> bool {
    is_ignored(event_path, ignore) && !is_explicit_file(event_path, explicit_files)
}

/// Returns true when `event_path` lives under any ignored root.
///
/// Matching uses two strategies:
/// 1. **Absolute roots** (e.g. canonicalized `dist/`): the event path
///    must literally start with the root. Handles "ignore this exact
///    output directory" cases.
/// 2. **Single-component relative roots** (e.g. `node_modules`,
///    `.git`, `target`): match if any component of the event path
///    equals that name. This is what makes `default_ignore_paths()`
///    work universally — `target` matches `/repo/target/...`,
///    `/repo/sub/target/...`, etc., regardless of where the watcher
///    cwd was when the path was registered.
fn is_ignored(event_path: &Path, ignore: &[PathBuf]) -> bool {
    if ignore.is_empty() {
        return false;
    }
    let canon = std::fs::canonicalize(event_path).ok();
    let candidate: &Path = canon.as_deref().unwrap_or(event_path);

    for root in ignore {
        if root.is_absolute() {
            if candidate.starts_with(root) {
                return true;
            }
            continue;
        }
        // Relative root: treat as a name to match against any path
        // component. Skips the absolute-prefix mismatch trap that
        // would otherwise let `target` / `node_modules` through.
        let mut components = root.components();
        let first = components.next();
        let only_one_component = components.next().is_none();
        if let (Some(first), true) = (first, only_one_component) {
            let name = first.as_os_str();
            if candidate.components().any(|c| c.as_os_str() == name) {
                return true;
            }
        } else if candidate.starts_with(root) {
            // Multi-component relative root — fall back to prefix.
            return true;
        }
    }
    false
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

/// Largest file the watcher will hash to detect a no-op change. Above this,
/// an event is always treated as a change — hashing a huge file on every event
/// would cost more than an occasional rebuild. Dev source files are tiny, so
/// this only guards pathological inputs.
const MAX_HASH_BYTES: u64 = 8 * 1024 * 1024;

/// Whether `path`'s content changed since the previous event, updating `cache`.
///
/// A path that cannot be read as a regular file within the size cap (deleted,
/// a directory, a permissions error, or oversized) is treated as **changed** so
/// deletions still trigger a rebuild and large files are never silently skipped.
fn content_changed(cache: &mut HashMap<PathBuf, u64>, path: &Path) -> bool {
    match hash_file(path) {
        Some(hash) => match cache.insert(path.to_path_buf(), hash) {
            Some(previous) => previous != hash,
            None => true,
        },
        None => {
            cache.remove(path);
            true
        }
    }
}

fn should_forward_path(
    cache: &mut HashMap<PathBuf, u64>,
    path: &Path,
    retry_unchanged: bool,
) -> bool {
    content_changed(cache, path) || retry_unchanged
}

/// Hash the full contents of `path`, or `None` if it is not a readable regular
/// file within [`MAX_HASH_BYTES`]. Uses the standard hasher — collision
/// resistance is irrelevant here; we only need "did these bytes change".
fn hash_file(path: &Path) -> Option<u64> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_HASH_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let mut hasher = DefaultHasher::new();
    hasher.write(&bytes);
    Some(hasher.finish())
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
    fn explicit_manifest_bypasses_ignored_output_directory() {
        let dir = tempfile::tempdir().unwrap();
        let dist = dir.path().join("dist");
        std::fs::create_dir(&dist).unwrap();
        let manifest = dist.join("webui-projection.json");
        let bundle = dist.join("index.js");
        std::fs::write(&manifest, "{}").unwrap();
        std::fs::write(&bundle, "export {};").unwrap();

        let ignore = vec![std::fs::canonicalize(&dist).unwrap()];
        let explicit = vec![std::fs::canonicalize(&manifest).unwrap()];
        assert!(!should_ignore_event(&manifest, &ignore, &explicit));
        assert!(should_ignore_event(&bundle, &ignore, &explicit));
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

    #[test]
    fn is_ignored_matches_relative_component_anywhere() {
        // `target` (single relative component) must match no matter
        // how deep — this is how default_ignore_paths() actually
        // works in the wild against absolute paths from notify.
        let ignore = vec![PathBuf::from("target")];
        assert!(is_ignored(Path::new("/repo/target/debug/build"), &ignore));
        assert!(is_ignored(
            Path::new("/repo/sub/crate/target/foo.rs"),
            &ignore
        ));
        assert!(!is_ignored(Path::new("/repo/src/target_name.rs"), &ignore));
    }

    #[test]
    fn is_ignored_handles_node_modules_and_git() {
        let ignore = default_ignore_paths();
        assert!(is_ignored(
            Path::new("/repo/node_modules/foo/index.js"),
            &ignore
        ));
        assert!(is_ignored(Path::new("/repo/.git/HEAD"), &ignore));
        assert!(!is_ignored(Path::new("/repo/src/index.ts"), &ignore));
    }

    #[test]
    fn content_changed_skips_identical_saves() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.css");
        std::fs::write(&file, "a { color: red; }").unwrap();
        let mut cache = HashMap::new();

        // First sighting → changed (nothing cached yet).
        assert!(content_changed(&mut cache, &file));
        // Re-saving identical bytes (repeated Ctrl+S) → no change → no rebuild.
        assert!(!content_changed(&mut cache, &file));
        assert!(!content_changed(&mut cache, &file));

        // A real edit → changed.
        std::fs::write(&file, "a { color: blue; }").unwrap();
        assert!(content_changed(&mut cache, &file));
        // Identical again → unchanged.
        assert!(!content_changed(&mut cache, &file));

        // Deletion → changed, so a rebuild can clear stale output.
        std::fs::remove_file(&file).unwrap();
        assert!(content_changed(&mut cache, &file));
        // The cache forgot it, so a later recreation is a fresh change.
        std::fs::write(&file, "a { color: blue; }").unwrap();
        assert!(content_changed(&mut cache, &file));
    }

    #[test]
    fn unchanged_save_can_retry_when_error_active() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.css");
        std::fs::write(&file, "a { color: red; }").unwrap();
        let mut cache = HashMap::new();

        assert!(should_forward_path(&mut cache, &file, false));
        assert!(!should_forward_path(&mut cache, &file, false));
        assert!(should_forward_path(&mut cache, &file, true));
    }
}
