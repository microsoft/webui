// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Debounced filesystem watcher.
//!
//! Wraps `notify-debouncer-mini` so callers don't need to deal with the
//! debouncer event type. Returns a [`WatcherHandle`] that owns the
//! background thread; **the handle must be kept alive** for the watcher
//! to run.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use anyhow::{Context, Result};
use notify::{
    event::{AccessKind, AccessMode},
    Event, EventKind,
};
use notify_debouncer_mini::{
    new_debouncer_opt, Config as DebouncerConfig, DebounceEventResult, DebouncedEvent, Debouncer,
};

#[path = "watch_backend.rs"]
mod backend;
#[path = "watch_hash.rs"]
mod hash;
#[path = "watch_subscriptions.rs"]
mod subscriptions;

#[cfg(test)]
#[path = "watch_hash_tests.rs"]
mod hash_tests;

use backend::ContentChangeWatcher;
use hash::{hash_file, HASH_BUFFER_SIZE};
use subscriptions::Subscriptions;

/// Owns the watcher background thread. Drop to stop watching.
///
/// `notify-debouncer-mini` spawns its background thread inside the
/// debouncer struct; dropping the struct kills the thread and discards
/// events still inside the debounce window. Consumers should hold this
/// handle for the lifetime of their server and call
/// [`WatcherHandle::retarget`] when their inputs change instead of
/// replacing it.
pub struct WatcherHandle {
    debouncer: Debouncer<ContentChangeWatcher>,
    filter: Arc<Mutex<EventFilter>>,
    subscriptions: Subscriptions,
}

impl WatcherHandle {
    /// Replace the watched roots, exact files, and ignored subtrees in place.
    ///
    /// A replacement watcher would discard events this one has observed but
    /// not yet debounced, losing edits made while a host refreshes its inputs.
    /// Retargeting keeps the debouncer and unchanged roots registered, so those
    /// events are still delivered. Content baselines restart as they would for
    /// a new watcher: exact files are re-hashed and other paths forward their
    /// next event.
    ///
    /// # Errors
    ///
    /// Returns the first error from a path that exists but cannot be watched.
    /// The remaining paths are still registered, including requested roots
    /// nested inside the failed path, and the failed path is retried on the
    /// next retarget.
    pub fn retarget(&mut self, targets: WatchTargets) -> Result<()> {
        let WatchTargets {
            paths,
            explicit_files,
            ignore,
        } = targets;
        // Canonicalize ignore paths once. Non-existent entries are kept as-is
        // so they still match if the path appears mid-session (e.g. a fresh
        // `dist/` created by the first build).
        let ignore = ignore
            .into_iter()
            .map(|path| std::fs::canonicalize(&path).unwrap_or(path))
            .collect();
        let explicit_files: Vec<PathBuf> = explicit_files
            .iter()
            .filter_map(|path| normalize_explicit_file(path))
            .collect();
        // Install the new filter before subscribing so events from new
        // subscriptions are judged against the new targets.
        self.filter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reset(ignore, explicit_files.clone());
        self.subscriptions
            .retarget(self.debouncer.watcher(), &paths, &explicit_files)
    }
}

fn is_read_only_access_event(event: &Event) -> bool {
    // inotify reports read-only IN_OPEN as Any; actual writes still emit
    // modification and close-write events.
    matches!(
        event.kind,
        EventKind::Access(
            AccessKind::Read
                | AccessKind::Open(AccessMode::Any | AccessMode::Read)
                | AccessKind::Close(AccessMode::Read)
        )
    )
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
    /// Missing parents are watched non-recursively. Directory changes are
    /// forwarded as dependent file paths so hosts can refresh their watches.
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

/// Inputs that [`WatcherHandle::retarget`] replaces on a running watcher.
///
/// Fields mean the same as their [`WatchConfig`] counterparts.
pub struct WatchTargets {
    /// Roots to watch recursively. Non-existent entries are skipped until a
    /// later retarget finds them.
    pub paths: Vec<PathBuf>,
    /// Exact files to watch through their parent directories, bypassing
    /// `ignore`.
    pub explicit_files: Vec<PathBuf>,
    /// Subtrees whose events are suppressed.
    pub ignore: Vec<PathBuf>,
}

/// Event filtering state shared by the debouncer thread and
/// [`WatcherHandle::retarget`].
struct EventFilter {
    ignore: Vec<PathBuf>,
    explicit_files: Vec<PathBuf>,
    content_hashes: HashMap<PathBuf, u64>,
    hash_buffer: [u8; HASH_BUFFER_SIZE],
}

impl EventFilter {
    fn new() -> Self {
        Self {
            ignore: Vec::new(),
            explicit_files: Vec::new(),
            content_hashes: HashMap::new(),
            hash_buffer: [0; HASH_BUFFER_SIZE],
        }
    }

    /// Filter by new targets and restart content baselines, as a new watcher
    /// would: exact files compare with their current bytes.
    fn reset(&mut self, ignore: Vec<PathBuf>, explicit_files: Vec<PathBuf>) {
        self.ignore = ignore;
        self.explicit_files = explicit_files;
        self.content_hashes.clear();
        for file in &self.explicit_files {
            content_changed(&mut self.content_hashes, file, &mut self.hash_buffer);
        }
    }

    /// Changed source paths in one debounce window.
    fn changed_paths(&mut self, events: &[DebouncedEvent], retry_unchanged: bool) -> Vec<PathBuf> {
        // Filter out ignored paths and dedupe (notify can emit
        // duplicate events for the same path within a window).
        let mut paths: Vec<PathBuf> = Vec::with_capacity(events.len());
        let mut seen: HashSet<PathBuf> = HashSet::with_capacity(events.len());
        for event in events {
            collect_event_paths(
                &event.path,
                &self.ignore,
                &self.explicit_files,
                &mut seen,
                &mut paths,
            );
        }
        // Recursive backends emit directory metadata events alongside
        // file writes. Existing directories are not source changes and
        // would otherwise retrigger a rebuild after every build; keep
        // missing paths so directory/file deletions still rebuild.
        paths.retain(|path| !path.is_dir());
        // Drop paths whose content is byte-identical to the last event.
        // Editors fire a write on every Ctrl+S even when nothing changed;
        // rebuilding then is pure wasted work, so a no-op save triggers no
        // rebuild at all in the clean state. When the caller reports that a
        // rebuild error is active, unchanged events are allowed through so a
        // no-op save can retry transient failures. Deletions and oversized
        // files always count as changed (see `content_changed`).
        paths.retain(|path| {
            should_forward_path(
                &mut self.content_hashes,
                path,
                retry_unchanged,
                &mut self.hash_buffer,
            )
        });
        paths
    }
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
/// optional `public/`) may not exist yet. Use [`WatcherHandle::retarget`]
/// to change the watched paths later.
///
/// # Errors
///
/// Returns an error if the watcher cannot be created or if a path that
/// exists cannot be watched (typically a permissions issue).
pub fn spawn_watcher<F>(cfg: WatchConfig, on_event: F) -> Result<WatcherHandle>
where
    F: Fn(Vec<PathBuf>) + Send + 'static,
{
    let WatchConfig {
        paths,
        explicit_files,
        ignore,
        debounce,
        retry_unchanged_when,
    } = cfg;
    let filter = Arc::new(Mutex::new(EventFilter::new()));
    let events_filter = Arc::clone(&filter);
    let notify_config = notify::Config::default().with_follow_symlinks(false);
    let debouncer_config = DebouncerConfig::default()
        .with_timeout(debounce)
        .with_notify_config(notify_config);
    let debouncer = new_debouncer_opt::<_, ContentChangeWatcher>(
        debouncer_config,
        move |res: DebounceEventResult| match res {
            Ok(events) => {
                // Ask the host before taking the filter lock.
                let retry_unchanged = retry_unchanged_when
                    .as_ref()
                    .is_some_and(|predicate| predicate());
                let paths = events_filter
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .changed_paths(&events, retry_unchanged);
                if !paths.is_empty() {
                    on_event(paths);
                }
            }
            Err(e) => {
                eprintln!("watcher error: {e:?}");
            }
        },
    )
    .context("Cannot start file watcher")?;

    let mut handle = WatcherHandle {
        debouncer,
        filter,
        subscriptions: Subscriptions::default(),
    };
    handle.retarget(WatchTargets {
        paths,
        explicit_files,
        ignore,
    })?;
    Ok(handle)
}

fn normalize_explicit_file(path: &Path) -> Option<PathBuf> {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return Some(canonical);
    }
    let path = std::path::absolute(path).ok()?;
    for ancestor in path.ancestors().skip(1) {
        if let Ok(canonical) = std::fs::canonicalize(ancestor) {
            return Some(canonical.join(path.strip_prefix(ancestor).ok()?));
        }
    }
    None
}

fn is_explicit_file(event_path: &Path, explicit_files: &[PathBuf]) -> bool {
    let normalized = std::fs::canonicalize(event_path).unwrap_or_else(|_| event_path.to_path_buf());
    explicit_files.iter().any(|path| path == &normalized)
}

fn should_ignore_event(event_path: &Path, ignore: &[PathBuf], explicit_files: &[PathBuf]) -> bool {
    is_ignored(event_path, ignore) && !is_explicit_file(event_path, explicit_files)
}

fn collect_event_paths(
    event_path: &Path,
    ignore: &[PathBuf],
    explicit_files: &[PathBuf],
    seen: &mut HashSet<PathBuf>,
    paths: &mut Vec<PathBuf>,
) {
    let is_directory = event_path.is_dir();
    if is_directory || !event_path.exists() {
        if let Some(directory) = normalize_explicit_file(event_path) {
            // Recursive backends can attach after a new directory is populated,
            // leaving its creation event as the only signal for included files.
            for file in explicit_files {
                if file != &directory && file.starts_with(&directory) && seen.insert(file.clone()) {
                    paths.push(file.clone());
                }
            }
        }
    }
    if !is_directory
        && !should_ignore_event(event_path, ignore, explicit_files)
        && seen.insert(event_path.to_path_buf())
    {
        paths.push(event_path.to_path_buf());
    }
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
            // Check the lexical event path before its canonical form. pnpm
            // dependencies are symlinks, so canonicalization removes the
            // `node_modules` component that identifies the ignored subtree.
            if event_path.components().any(|c| c.as_os_str() == name)
                || candidate.components().any(|c| c.as_os_str() == name)
            {
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

/// Whether `path`'s content changed since the previous event, updating `cache`.
///
/// A path that cannot be read as a regular file within the size cap (deleted,
/// a directory, a permissions error, or oversized) is treated as **changed** so
/// deletions still trigger a rebuild and large files are never silently skipped.
fn content_changed(
    cache: &mut HashMap<PathBuf, u64>,
    path: &Path,
    buffer: &mut [u8; HASH_BUFFER_SIZE],
) -> bool {
    match hash_file(path, buffer) {
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
    buffer: &mut [u8; HASH_BUFFER_SIZE],
) -> bool {
    content_changed(cache, path, buffer) || retry_unchanged
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use notify_debouncer_mini::DebouncedEventKind;
    use std::sync::mpsc::{self, RecvTimeoutError};

    fn event(path: &Path) -> DebouncedEvent {
        DebouncedEvent::new(path.to_path_buf(), DebouncedEventKind::Any)
    }

    #[test]
    fn reset_restarts_content_baselines_like_a_new_watcher() {
        let dir = tempfile::tempdir().unwrap();
        let exact = dir.path().join("state.json");
        let source = dir.path().join("page.md");
        std::fs::write(&exact, "1").unwrap();
        std::fs::write(&source, "# Page").unwrap();
        let mut filter = EventFilter::new();
        filter.reset(Vec::new(), vec![exact.clone()]);

        assert!(filter.changed_paths(&[event(&exact)], false).is_empty());
        assert_eq!(
            filter.changed_paths(&[event(&source)], false),
            std::slice::from_ref(&source)
        );
        assert!(filter.changed_paths(&[event(&source)], false).is_empty());

        std::fs::write(&exact, "2").unwrap();
        filter.reset(Vec::new(), vec![exact.clone()]);
        assert!(filter.changed_paths(&[event(&exact)], false).is_empty());
        assert_eq!(filter.changed_paths(&[event(&source)], false), [source]);
    }

    #[test]
    fn retarget_delivers_edits_observed_before_it() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let source = dir.path().join("page.md");
        std::fs::write(&source, "# One")?;
        let (sender, receiver) = mpsc::channel();
        let mut watcher = spawn_watcher(
            WatchConfig {
                paths: vec![dir.path().to_path_buf()],
                explicit_files: Vec::new(),
                ignore: Vec::new(),
                debounce: Duration::from_millis(50),
                retry_unchanged_when: None,
            },
            move |paths| {
                let _ = sender.send(paths);
            },
        )?;

        std::fs::write(&source, "# Two")?;
        // A host refreshing its inputs right after the edit must not drop it.
        watcher.retarget(WatchTargets {
            paths: vec![dir.path().to_path_buf()],
            explicit_files: Vec::new(),
            ignore: Vec::new(),
        })?;
        let changed = receiver.recv_timeout(Duration::from_secs(5))?;
        let source = std::fs::canonicalize(&source)?;
        assert!(
            changed
                .iter()
                .any(|path| std::fs::canonicalize(path).is_ok_and(|path| path == source)),
            "missing {} in {changed:?}",
            source.display()
        );
        Ok(())
    }

    #[test]
    fn retarget_moves_exact_file_subscriptions() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let first = dir.path().join("target/first/state.json");
        let second = dir.path().join("target/second/state.json");
        for file in [&first, &second] {
            std::fs::create_dir_all(file.parent().context("state parent")?)?;
            std::fs::write(file, "1")?;
        }
        let ignore = || vec![PathBuf::from("target")];
        let (sender, receiver) = mpsc::channel();
        let mut watcher = spawn_watcher(
            WatchConfig {
                paths: Vec::new(),
                explicit_files: vec![first.clone()],
                ignore: ignore(),
                debounce: Duration::from_millis(25),
                retry_unchanged_when: None,
            },
            move |paths| {
                let _ = sender.send(paths);
            },
        )?;

        watcher.retarget(WatchTargets {
            paths: Vec::new(),
            explicit_files: vec![second.clone()],
            ignore: ignore(),
        })?;
        std::fs::write(&first, "2")?;
        let stale = receiver.recv_timeout(Duration::from_millis(500));
        assert!(
            matches!(stale, Err(RecvTimeoutError::Timeout)),
            "a released include must not rebuild: {stale:?}"
        );
        std::fs::write(&second, "2")?;
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(5))?,
            [std::fs::canonicalize(&second)?]
        );
        Ok(())
    }

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
    fn explicit_file_tracks_missing_parent_directories() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("target/new/state.json");
        let normalized = normalize_explicit_file(&file)
            .ok_or_else(|| std::io::Error::other("missing include was discarded"))?;
        assert_eq!(
            normalized,
            std::fs::canonicalize(dir.path())?.join("target/new/state.json")
        );
        std::fs::create_dir_all(
            file.parent()
                .ok_or_else(|| std::io::Error::other("missing state parent"))?,
        )?;
        std::fs::write(&file, "{}")?;
        assert!(!should_ignore_event(
            &file,
            &[PathBuf::from("target")],
            &[normalized]
        ));
        Ok(())
    }

    #[test]
    fn directory_event_rechecks_explicit_files_created_before_recursive_watch() {
        let dir = tempfile::tempdir().unwrap();
        let directory = dir.path().join("target/new");
        let file = directory.join("state.json");
        let explicit = vec![normalize_explicit_file(&file).unwrap()];
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&file, "{}").unwrap();
        let mut seen = HashSet::new();
        let mut paths = Vec::new();
        collect_event_paths(
            &directory,
            &[PathBuf::from("target")],
            &explicit,
            &mut seen,
            &mut paths,
        );
        assert_eq!(paths, explicit);
    }

    #[test]
    fn directory_rechecks_dedupe_and_skip_unchanged_explicit_files() {
        let dir = tempfile::tempdir().unwrap();
        let directory = dir.path().join("target/new");
        std::fs::create_dir_all(&directory).unwrap();
        let file = directory.join("state.json");
        std::fs::write(&file, "{}").unwrap();
        let explicit = vec![normalize_explicit_file(&file).unwrap()];
        let mut hashes = HashMap::new();
        let mut buffer = [0_u8; HASH_BUFFER_SIZE];
        assert!(content_changed(&mut hashes, &explicit[0], &mut buffer));
        let mut seen = HashSet::new();
        let mut paths = Vec::new();
        for _ in 0..2 {
            collect_event_paths(
                &directory,
                &[PathBuf::from("target")],
                &explicit,
                &mut seen,
                &mut paths,
            );
        }
        assert_eq!(paths, explicit);
        assert!(!should_forward_path(
            &mut hashes,
            &paths[0],
            false,
            &mut buffer
        ));
        std::fs::write(&file, "{\"count\":7}").unwrap();
        assert!(should_forward_path(
            &mut hashes,
            &paths[0],
            false,
            &mut buffer
        ));
    }

    #[test]
    fn directory_events_ignore_unrelated_files_and_recheck_removed_parents() {
        let dir = tempfile::tempdir().unwrap();
        let directory = dir.path().join("target/new");
        let unrelated = dir.path().join("target/other");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::create_dir_all(&unrelated).unwrap();
        let file = directory.join("state.json");
        std::fs::write(&file, "{}").unwrap();
        let explicit = vec![normalize_explicit_file(&file).unwrap()];
        let ignore = vec![PathBuf::from("target")];
        let mut seen = HashSet::new();
        let mut paths = Vec::new();
        collect_event_paths(&unrelated, &ignore, &explicit, &mut seen, &mut paths);
        assert!(paths.is_empty());
        std::fs::remove_file(&file).unwrap();
        std::fs::remove_dir(&directory).unwrap();
        collect_event_paths(&directory, &ignore, &explicit, &mut seen, &mut paths);
        assert_eq!(paths, explicit);
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
    fn read_only_access_events_are_not_content_changes() {
        use notify::event::ModifyKind;

        for kind in [
            AccessKind::Read,
            AccessKind::Open(AccessMode::Any),
            AccessKind::Open(AccessMode::Read),
            AccessKind::Close(AccessMode::Read),
        ] {
            assert!(is_read_only_access_event(&Event::new(EventKind::Access(
                kind
            ))));
        }
        for kind in [
            AccessKind::Any,
            AccessKind::Open(AccessMode::Write),
            AccessKind::Close(AccessMode::Write),
        ] {
            assert!(!is_read_only_access_event(&Event::new(EventKind::Access(
                kind
            ))));
        }
        assert!(!is_read_only_access_event(&Event::new(EventKind::Modify(
            ModifyKind::Any
        ))));
    }

    #[cfg(unix)]
    #[test]
    fn is_ignored_matches_node_modules_symlink_path() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let package = dir.path().join("packages/example");
        let node_modules = dir.path().join("node_modules");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::create_dir(&node_modules).unwrap();
        std::fs::write(package.join("index.js"), "export {};").unwrap();
        symlink(&package, node_modules.join("example")).unwrap();

        let linked_file = node_modules.join("example/index.js");
        let canonical = std::fs::canonicalize(&linked_file).unwrap();
        assert!(!canonical
            .components()
            .any(|component| component.as_os_str() == "node_modules"));
        assert!(is_ignored(&linked_file, &[PathBuf::from("node_modules")]));
    }

    #[test]
    fn content_changed_skips_identical_saves() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.css");
        std::fs::write(&file, "a { color: red; }").unwrap();
        let mut cache = HashMap::new();
        let mut buffer = [0_u8; HASH_BUFFER_SIZE];

        // First sighting → changed (nothing cached yet).
        assert!(content_changed(&mut cache, &file, &mut buffer));
        // Re-saving identical bytes (repeated Ctrl+S) → no change → no rebuild.
        assert!(!content_changed(&mut cache, &file, &mut buffer));
        assert!(!content_changed(&mut cache, &file, &mut buffer));

        // A real edit → changed.
        std::fs::write(&file, "a { color: blue; }").unwrap();
        assert!(content_changed(&mut cache, &file, &mut buffer));
        // Identical again → unchanged.
        assert!(!content_changed(&mut cache, &file, &mut buffer));

        // Deletion → changed, so a rebuild can clear stale output.
        std::fs::remove_file(&file).unwrap();
        assert!(content_changed(&mut cache, &file, &mut buffer));
        // The cache forgot it, so a later recreation is a fresh change.
        std::fs::write(&file, "a { color: blue; }").unwrap();
        assert!(content_changed(&mut cache, &file, &mut buffer));
    }

    #[test]
    fn unchanged_save_can_retry_when_error_active() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.css");
        std::fs::write(&file, "a { color: red; }").unwrap();
        let mut cache = HashMap::new();
        let mut buffer = [0_u8; HASH_BUFFER_SIZE];

        assert!(should_forward_path(&mut cache, &file, false, &mut buffer));
        assert!(!should_forward_path(&mut cache, &file, false, &mut buffer));
        assert!(should_forward_path(&mut cache, &file, true, &mut buffer));
        assert!(!should_forward_path(&mut cache, &file, false, &mut buffer));

        std::fs::write(&file, "a { color: blue; }").unwrap();
        assert!(should_forward_path(&mut cache, &file, true, &mut buffer));
        assert!(!should_forward_path(&mut cache, &file, false, &mut buffer));
    }

    #[test]
    fn unhashable_paths_forget_cached_content() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("a.css");
        let mut cache = HashMap::new();
        let mut buffer = [0_u8; HASH_BUFFER_SIZE];

        for replacement in ["deleted", "directory", "oversized"] {
            std::fs::write(&file, "original")?;
            assert!(content_changed(&mut cache, &file, &mut buffer));
            assert!(!content_changed(&mut cache, &file, &mut buffer));

            std::fs::remove_file(&file)?;
            match replacement {
                "directory" => std::fs::create_dir(&file)?,
                "oversized" => std::fs::File::create(&file)?.set_len(8 * 1024 * 1024 + 1)?,
                _ => {}
            }
            for _ in 0..2 {
                assert!(content_changed(&mut cache, &file, &mut buffer));
                assert!(!cache.contains_key(&file));
            }
            match replacement {
                "directory" => std::fs::remove_dir(&file)?,
                "oversized" => std::fs::remove_file(&file)?,
                _ => {}
            }
        }
        std::fs::write(&file, "original")?;
        assert!(content_changed(&mut cache, &file, &mut buffer));
        Ok(())
    }

    #[test]
    fn hash_buffer_is_reused_across_files_and_event_batches() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = [
            dir.path().join("large.css"),
            dir.path().join("empty.css"),
            dir.path().join("small.css"),
        ];
        for (path, size) in paths.iter().zip([3 * HASH_BUFFER_SIZE + 1, 0, 7]) {
            std::fs::write(path, vec![b'x'; size])?;
        }
        let mut cache = HashMap::new();
        let mut buffer = [0_u8; HASH_BUFFER_SIZE];
        for changed in [true, false, false] {
            for path in &paths {
                assert_eq!(content_changed(&mut cache, path, &mut buffer), changed);
            }
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_symlink_loop_forgets_cached_content() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("unreadable.css");
        std::fs::write(&file, "original")?;
        let mut cache = HashMap::new();
        let mut buffer = [0_u8; HASH_BUFFER_SIZE];
        assert!(content_changed(&mut cache, &file, &mut buffer));
        std::fs::remove_file(&file)?;
        std::os::unix::fs::symlink("unreadable.css", &file)?;
        assert!(content_changed(&mut cache, &file, &mut buffer));
        assert!(!cache.contains_key(&file));
        std::fs::remove_file(&file)?;
        std::fs::write(&file, "original")?;
        assert!(content_changed(&mut cache, &file, &mut buffer));
        Ok(())
    }
}
