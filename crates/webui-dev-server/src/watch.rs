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
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use notify::{
    event::{AccessKind, AccessMode},
    Event, EventHandler, EventKind, RecommendedWatcher, RecursiveMode, Watcher, WatcherKind,
};
use notify_debouncer_mini::{
    new_debouncer_opt, Config as DebouncerConfig, DebounceEventResult, Debouncer,
};

#[path = "watch_hash.rs"]
mod hash;

#[path = "watch_snapshot.rs"]
mod snapshot;

#[cfg(test)]
#[path = "watch_hash_tests.rs"]
mod hash_tests;

use hash::{hash_file, HASH_BUFFER_SIZE};

/// Owns the watcher background thread. Drop to stop watching.
///
/// `notify-debouncer-mini` spawns its background thread inside the
/// debouncer struct; dropping the struct kills the thread. Consumers
/// should hold this handle for the lifetime of their server.
pub struct WatcherHandle {
    _debouncer: Debouncer<ContentChangeWatcher>,
}

struct ContentChangeWatcher(RecommendedWatcher);

impl Watcher for ContentChangeWatcher {
    fn new<F: EventHandler>(mut event_handler: F, config: notify::Config) -> notify::Result<Self> {
        RecommendedWatcher::new(
            move |event| {
                if matches!(&event, Ok(event) if is_read_only_access_event(event)) {
                    return;
                }
                event_handler.handle_event(event);
            },
            config,
        )
        .map(Self)
    }

    fn watch(&mut self, path: &Path, recursive_mode: RecursiveMode) -> notify::Result<()> {
        self.0.watch(path, recursive_mode)
    }

    fn unwatch(&mut self, path: &Path) -> notify::Result<()> {
        self.0.unwatch(path)
    }

    fn configure(&mut self, config: notify::Config) -> notify::Result<bool> {
        self.0.configure(config)
    }

    fn kind() -> WatcherKind {
        RecommendedWatcher::kind()
    }
}

fn is_read_only_access_event(event: &Event) -> bool {
    matches!(
        event.kind,
        EventKind::Access(
            AccessKind::Read
                | AccessKind::Open(AccessMode::Read)
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
    pub explicit_files: Vec<PathBuf>,
    /// Subtrees to ignore. An event is suppressed when its path lives
    /// underneath any entry here. Typical values: the build's `out_dir`,
    /// `node_modules`, `.git`, `target`.
    ///
    /// Bare relative names match subtrees beneath each watched root, not its
    /// ancestors. Existing explicit paths are canonicalized at registration;
    /// use absolute paths for output locations that may be created later.
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
    // Bare relative names are subtree filters, even when a matching directory
    // exists in cwd. Explicit paths identify one output location.
    let ignore: Vec<PathBuf> = cfg
        .ignore
        .iter()
        .map(|p| {
            if p.is_relative()
                && p.components().count() == 1
                && matches!(p.components().next(), Some(std::path::Component::Normal(_)))
            {
                p.clone()
            } else {
                std::fs::canonicalize(p).unwrap_or_else(|_| p.clone())
            }
        })
        .collect();
    let explicit_files: Vec<PathBuf> = cfg
        .explicit_files
        .iter()
        .filter_map(|path| normalize_explicit_file(path))
        .collect();

    let roots = cfg
        .paths
        .iter()
        .filter(|path| path.exists())
        .map(|path| {
            std::fs::canonicalize(path)
                .with_context(|| format!("Cannot resolve watched root {}", path.display()))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut content_hashes = snapshot::initial_hashes(&roots, &ignore, &explicit_files)?;
    let mut hash_buffer = [0_u8; HASH_BUFFER_SIZE];
    let retry_unchanged_when = cfg.retry_unchanged_when.clone();
    let explicit_filter = explicit_files.clone();
    let event_roots = roots.clone();
    let notify_config = notify::Config::default().with_follow_symlinks(false);
    let debouncer_config = DebouncerConfig::default()
        .with_timeout(cfg.debounce)
        .with_notify_config(notify_config);
    let mut debouncer = new_debouncer_opt::<_, ContentChangeWatcher>(
        debouncer_config,
        move |res: DebounceEventResult| match res {
            Ok(events) => {
                // Filter out ignored paths and dedupe (notify can emit
                // duplicate events for the same path within a window).
                let mut paths: Vec<PathBuf> = Vec::with_capacity(events.len());
                let mut seen: HashSet<PathBuf> = HashSet::with_capacity(events.len());
                for e in events {
                    if should_ignore_event(&e.path, &ignore, &explicit_filter, &event_roots) {
                        continue;
                    }
                    if seen.insert(e.path.clone()) {
                        paths.push(e.path);
                    }
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
                let retry_unchanged = retry_unchanged_when
                    .as_ref()
                    .is_some_and(|predicate| predicate());
                paths.retain(|path| {
                    should_forward_path(
                        &mut content_hashes,
                        path,
                        retry_unchanged,
                        &mut hash_buffer,
                    )
                });
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

    for root in &roots {
        debouncer
            .watcher()
            .watch(root, RecursiveMode::Recursive)
            .with_context(|| format!("Cannot watch {}", root.display()))?;
    }
    let mut explicit_parents = HashSet::new();
    for file in &explicit_files {
        if roots.iter().any(|root| file.starts_with(root)) {
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

fn should_ignore_event(
    event_path: &Path,
    ignore: &[PathBuf],
    explicit_files: &[PathBuf],
    roots: &[PathBuf],
) -> bool {
    if is_explicit_file(event_path, explicit_files) {
        return false;
    }
    if !roots.iter().any(|root| event_path.starts_with(root)) {
        return true;
    }
    is_ignored_within(event_path, ignore, roots)
}

/// Returns true when `event_path` lives under any ignored root.
///
/// Matching uses two strategies:
/// 1. **Absolute roots** (e.g. canonicalized `dist/`): the event path
///    or its resolved target must start with the root. Handles "ignore this exact
///    output directory" cases.
/// 2. **Single-component relative roots** (e.g. `node_modules`,
///    `.git`, `target`): match if any component of the event path
///    equals that name beneath the most specific watched root. Explicit roots
///    remain watchable even when an ancestor is named `target` or `node_modules`.
fn is_ignored_within(event_path: &Path, ignore: &[PathBuf], roots: &[PathBuf]) -> bool {
    if ignore.is_empty() {
        return false;
    }
    let canon = std::fs::canonicalize(event_path).ok();
    let candidate: &Path = canon.as_deref().unwrap_or(event_path);

    for root in ignore {
        if root.is_absolute() {
            if event_path.starts_with(root) || candidate.starts_with(root) {
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
            if relative_to_root(event_path, roots)
                .components()
                .any(|c| c.as_os_str() == name)
                || relative_to_root(candidate, roots)
                    .components()
                    .any(|c| c.as_os_str() == name)
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

fn relative_to_root<'a>(path: &'a Path, roots: &[PathBuf]) -> &'a Path {
    roots
        .iter()
        .filter_map(|root| path.strip_prefix(root).ok())
        .min_by_key(|relative| relative.components().count())
        .unwrap_or(path)
}

#[cfg(test)]
fn is_ignored(path: &Path, ignore: &[PathBuf]) -> bool {
    is_ignored_within(path, ignore, &[])
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
        assert!(!should_ignore_event(&manifest, &ignore, &explicit, &[]));
        assert!(should_ignore_event(&bundle, &ignore, &explicit, &[]));
    }

    #[test]
    fn explicitly_watched_root_is_not_ignored_because_of_its_ancestors() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("target").join("app");
        std::fs::create_dir_all(app.join("target")).unwrap();
        std::fs::write(app.join("index.ts"), "source").unwrap();
        std::fs::write(app.join("target").join("generated.ts"), "output").unwrap();
        let app = app.canonicalize().unwrap();
        let roots = vec![app.clone()];
        let ignore = default_ignore_paths();
        assert!(!should_ignore_event(
            &app.join("index.ts"),
            &ignore,
            &[],
            &roots
        ));
        assert!(should_ignore_event(
            &app.join("target").join("generated.ts"),
            &ignore,
            &[],
            &roots
        ));
    }

    #[test]
    fn watching_an_external_file_does_not_forward_its_unrelated_siblings() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("app");
        std::fs::create_dir(&app).unwrap();
        let state = dir.path().join("state.json");
        let unrelated = dir.path().join("unrelated.json");
        std::fs::write(&state, "{}").unwrap();
        std::fs::write(&unrelated, "{}").unwrap();
        let roots = vec![app.canonicalize().unwrap()];
        let files = vec![state.canonicalize().unwrap()];
        assert!(!should_ignore_event(&state, &[], &files, &roots));
        assert!(should_ignore_event(&unrelated, &[], &files, &roots));
    }

    #[test]
    fn relative_roots_and_ignore_names_work_through_the_actual_watcher() {
        let directory = tempfile::Builder::new()
            .prefix("watch-root-")
            .tempdir_in(".")
            .unwrap();
        let name = PathBuf::from(directory.path().file_name().unwrap());
        let file = directory.path().join("index.ts");
        let ignored = directory.path().join(&name).join("ignored.ts");
        std::fs::create_dir(ignored.parent().unwrap()).unwrap();
        std::fs::write(&file, "before").unwrap();
        std::fs::write(&ignored, "before").unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let _watcher = spawn_watcher(
            WatchConfig {
                paths: vec![directory.path().to_path_buf()],
                explicit_files: Vec::new(),
                ignore: vec![name],
                debounce: Duration::from_millis(10),
                retry_unchanged_when: None,
            },
            move |paths| {
                let _ = sender.send(paths);
            },
        )
        .unwrap();
        std::fs::write(&ignored, "ignored").unwrap();
        assert!(receiver.recv_timeout(Duration::from_millis(100)).is_err());
        std::fs::write(&file, "after").unwrap();
        let changed = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(changed.contains(&file.canonicalize().unwrap()));
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
