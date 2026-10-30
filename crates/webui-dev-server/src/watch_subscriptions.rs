// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;
use notify::{RecursiveMode, Watcher};

/// A recursive root registered with the backend.
struct Root {
    /// Path passed to the backend, which reports events under this spelling.
    path: PathBuf,
    /// Canonical form used to compare roots and exact files.
    canonical: PathBuf,
}

/// Subscriptions currently registered with a watcher backend.
#[derive(Default)]
pub(super) struct Subscriptions {
    roots: Vec<Root>,
    explicit_parents: HashSet<PathBuf>,
}

impl Subscriptions {
    /// Register existing `paths` recursively and the nearest existing parents
    /// of normalized `explicit_files`, releasing registrations that are no
    /// longer needed.
    ///
    /// Unchanged roots stay registered so events already observed under them
    /// are still delivered. A requested root nested inside another requested
    /// root is registered only when that ancestor cannot be watched. Exact-file
    /// parents are always re-registered: some backends drop a subscription
    /// without an event when its directory is deleted, and generated
    /// directories are routinely recreated.
    ///
    /// A path that cannot be watched does not stop the others from being
    /// registered; the first failure is returned afterwards.
    pub(super) fn retarget(
        &mut self,
        watcher: &mut dyn Watcher,
        paths: &[PathBuf],
        explicit_files: &[PathBuf],
    ) -> Result<()> {
        for parent in self.explicit_parents.drain() {
            // A deleted directory may already have released its subscription.
            let _ = watcher.unwatch(&parent);
        }
        let requested = existing_roots(paths);
        // Release roots before adding new ones: removing a recursive root also
        // removes subscriptions a new nested root would share with it.
        self.roots.retain(|root| {
            let keep = is_outermost(&requested, &root.canonical);
            if !keep {
                let _ = watcher.unwatch(&root.path);
            }
            keep
        });
        let mut failure = None;
        // Ancestors come first, so nested roots are reached only when none of
        // their requested ancestors could be watched.
        for root in requested {
            if self
                .roots
                .iter()
                .any(|current| root.canonical.starts_with(&current.canonical))
            {
                continue;
            }
            let watched = watcher.watch(&root.path, RecursiveMode::Recursive);
            if keep_first_failure(&mut failure, watched, &root.path) {
                self.roots.push(root);
            } else {
                // Release what a partial recursive registration left behind.
                let _ = watcher.unwatch(&root.path);
            }
        }
        watch_explicit_parents(
            watcher,
            explicit_files,
            &self.roots,
            &mut self.explicit_parents,
            &mut failure,
        );
        failure.map_or(Ok(()), Err)
    }
}

/// Whether `result` succeeded, recording the first failure.
fn keep_first_failure(
    failure: &mut Option<anyhow::Error>,
    result: notify::Result<()>,
    path: &Path,
) -> bool {
    let Err(error) = result else {
        return true;
    };
    if failure.is_none() {
        *failure =
            Some(anyhow::Error::new(error).context(format!("Cannot watch {}", path.display())));
    }
    false
}

/// Existing `paths` without duplicates, ancestors before their descendants.
fn existing_roots(paths: &[PathBuf]) -> Vec<Root> {
    let mut roots: Vec<Root> = Vec::with_capacity(paths.len());
    for path in paths {
        if !path.exists() {
            continue;
        }
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        if roots.iter().any(|root| root.canonical == canonical) {
            continue;
        }
        roots.push(Root {
            path: path.clone(),
            canonical,
        });
    }
    roots.sort_by_key(|root| root.canonical.components().count());
    roots
}

/// Whether `path` is requested and not nested inside another requested root.
fn is_outermost(requested: &[Root], path: &Path) -> bool {
    requested.iter().any(|root| root.canonical == path)
        && !requested
            .iter()
            .any(|root| root.canonical != path && path.starts_with(&root.canonical))
}

fn watch_explicit_parents(
    watcher: &mut dyn Watcher,
    explicit_files: &[PathBuf],
    roots: &[Root],
    registered: &mut HashSet<PathBuf>,
    failure: &mut Option<anyhow::Error>,
) {
    for file in explicit_files {
        if roots.iter().any(|root| file.starts_with(&root.canonical)) {
            continue;
        }
        while let Some(parent) = file.ancestors().skip(1).find(|parent| parent.is_dir()) {
            if registered.contains(parent)
                || !keep_first_failure(
                    failure,
                    watcher.watch(parent, RecursiveMode::NonRecursive),
                    parent,
                )
            {
                break;
            }
            registered.insert(parent.to_path_buf());
            // A generator can create more parents during registration. Advance
            // before returning without subscribing to unrelated descendants.
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::super::normalize_explicit_file;
    use super::*;
    use notify::{EventHandler, RecommendedWatcher, WatcherKind};

    #[derive(Debug, PartialEq)]
    enum Call {
        Watch(PathBuf, RecursiveMode),
        Unwatch(PathBuf),
    }

    #[derive(Default)]
    struct RecordingWatcher {
        calls: Vec<Call>,
        denied: Option<PathBuf>,
    }

    impl Watcher for RecordingWatcher {
        fn new<F: EventHandler>(_: F, _: notify::Config) -> notify::Result<Self> {
            Ok(Self::default())
        }

        fn watch(&mut self, path: &Path, mode: RecursiveMode) -> notify::Result<()> {
            if self.denied.as_deref() == Some(path) {
                return Err(notify::Error::generic("permission denied"));
            }
            self.calls.push(Call::Watch(path.to_path_buf(), mode));
            Ok(())
        }

        fn unwatch(&mut self, path: &Path) -> notify::Result<()> {
            self.calls.push(Call::Unwatch(path.to_path_buf()));
            Ok(())
        }

        fn kind() -> WatcherKind {
            RecommendedWatcher::kind()
        }
    }

    #[test]
    fn missing_include_does_not_recursively_watch_ignored_ancestor() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        std::fs::create_dir_all(target.join("debug/build/unrelated")).unwrap();
        let file = normalize_explicit_file(&target.join("generated/new/state.json")).unwrap();
        let mut watcher = RecordingWatcher::default();
        let mut registered = HashSet::new();
        let mut failure = None;
        watch_explicit_parents(&mut watcher, &[file], &[], &mut registered, &mut failure);
        assert!(failure.is_none());
        assert_eq!(
            watcher.calls,
            vec![Call::Watch(
                std::fs::canonicalize(target).unwrap(),
                RecursiveMode::NonRecursive
            )]
        );
    }

    #[test]
    fn retarget_registers_remaining_paths_after_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let denied = dir.path().join("denied");
        let content = dir.path().join("content");
        let state = dir.path().join("state");
        for directory in [&denied, &content, &state] {
            std::fs::create_dir(directory).unwrap();
        }
        let file = normalize_explicit_file(&state.join("page.json")).unwrap();
        let mut watcher = RecordingWatcher {
            denied: Some(denied.clone()),
            ..RecordingWatcher::default()
        };
        let mut subscriptions = Subscriptions::default();

        let error = subscriptions
            .retarget(&mut watcher, &[denied.clone(), content.clone()], &[file])
            .unwrap_err();
        assert!(error.to_string().contains("denied"), "{error:#}");
        assert_eq!(
            watcher.calls,
            vec![
                // Releases anything a partial recursive registration left.
                Call::Unwatch(denied.clone()),
                Call::Watch(content, RecursiveMode::Recursive),
                Call::Watch(
                    std::fs::canonicalize(&state).unwrap(),
                    RecursiveMode::NonRecursive
                ),
            ]
        );

        // The failed root is retried on the next retarget.
        watcher.denied = None;
        watcher.calls.clear();
        subscriptions
            .retarget(&mut watcher, std::slice::from_ref(&denied), &[])
            .unwrap();
        assert!(watcher
            .calls
            .contains(&Call::Watch(denied, RecursiveMode::Recursive)));
    }

    #[test]
    fn retarget_keeps_unchanged_roots_and_refreshes_exact_file_parents() {
        let dir = tempfile::tempdir().unwrap();
        let content = dir.path().join("content");
        let state = dir.path().join("target/state");
        std::fs::create_dir_all(&content).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        let file = normalize_explicit_file(&state.join("page.json")).unwrap();
        let parent = std::fs::canonicalize(&state).unwrap();
        let mut watcher = RecordingWatcher::default();
        let mut subscriptions = Subscriptions::default();

        subscriptions
            .retarget(
                &mut watcher,
                std::slice::from_ref(&content),
                std::slice::from_ref(&file),
            )
            .unwrap();
        assert_eq!(
            watcher.calls,
            vec![
                Call::Watch(content.clone(), RecursiveMode::Recursive),
                Call::Watch(parent.clone(), RecursiveMode::NonRecursive),
            ]
        );

        watcher.calls.clear();
        subscriptions
            .retarget(
                &mut watcher,
                std::slice::from_ref(&content),
                std::slice::from_ref(&file),
            )
            .unwrap();
        assert_eq!(
            watcher.calls,
            vec![
                Call::Unwatch(parent.clone()),
                Call::Watch(parent, RecursiveMode::NonRecursive),
            ]
        );
    }

    #[test]
    fn retarget_releases_replaced_roots_before_watching_their_ancestor() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("docs");
        let nested = parent.join("content");
        std::fs::create_dir_all(&nested).unwrap();
        let mut watcher = RecordingWatcher::default();
        let mut subscriptions = Subscriptions::default();

        subscriptions
            .retarget(&mut watcher, &[nested.clone(), nested.clone()], &[])
            .unwrap();
        assert_eq!(
            watcher.calls,
            vec![Call::Watch(nested.clone(), RecursiveMode::Recursive)]
        );

        watcher.calls.clear();
        subscriptions
            .retarget(&mut watcher, &[nested.clone(), parent.clone()], &[])
            .unwrap();
        assert_eq!(
            watcher.calls,
            vec![
                Call::Unwatch(nested),
                Call::Watch(parent, RecursiveMode::Recursive),
            ]
        );
    }

    /// Subscriptions left active by the recorded calls.
    fn active(calls: &[Call]) -> Vec<&Path> {
        let mut active: Vec<&Path> = Vec::new();
        for call in calls {
            match call {
                Call::Watch(path, _) => active.push(path.as_path()),
                Call::Unwatch(path) => active.retain(|current| *current != path.as_path()),
            }
        }
        active
    }

    #[test]
    fn retarget_keeps_nested_roots_when_their_ancestor_cannot_be_watched() {
        let dir = tempfile::tempdir().unwrap();
        let docs = dir.path().join("docs");
        let content = docs.join("content");
        std::fs::create_dir_all(&content).unwrap();
        let requested = [content.clone(), docs.clone()];
        let mut watcher = RecordingWatcher::default();
        let mut subscriptions = Subscriptions::default();
        subscriptions
            .retarget(&mut watcher, std::slice::from_ref(&content), &[])
            .unwrap();

        watcher.denied = Some(docs.clone());
        let error = subscriptions
            .retarget(&mut watcher, &requested, &[])
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("Cannot watch {}", docs.display())
        );
        assert_eq!(active(&watcher.calls), [content.as_path()]);

        // The ancestor is retried and replaces its fallback once it is watchable.
        watcher.denied = None;
        subscriptions
            .retarget(&mut watcher, &requested, &[])
            .unwrap();
        assert_eq!(active(&watcher.calls), [docs.as_path()]);
    }

    #[test]
    fn retarget_follows_roots_that_disappear_and_return() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("public");
        let mut watcher = RecordingWatcher::default();
        let mut subscriptions = Subscriptions::default();
        let roots = std::slice::from_ref(&public);

        subscriptions.retarget(&mut watcher, roots, &[]).unwrap();
        assert!(watcher.calls.is_empty());

        std::fs::create_dir(&public).unwrap();
        subscriptions.retarget(&mut watcher, roots, &[]).unwrap();
        std::fs::remove_dir(&public).unwrap();
        subscriptions.retarget(&mut watcher, roots, &[]).unwrap();
        assert_eq!(
            watcher.calls,
            vec![
                Call::Watch(public.clone(), RecursiveMode::Recursive),
                Call::Unwatch(public),
            ]
        );
    }
}
