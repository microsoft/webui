// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use notify::{
    Config, Event, EventHandler, RecommendedWatcher, RecursiveMode, Watcher, WatcherKind,
};

use super::is_read_only_access_event;

type SharedHandler = Arc<Mutex<dyn EventHandler>>;

/// Notify backends that feed one debouncer.
///
/// Recursive roots and exact-file parents use separate backends. Exact-file
/// parents are re-registered on every retarget, and FSEvents applies each
/// subscription change by restarting its stream, which would also drop root
/// events during the restart.
pub(super) struct ContentChangeWatcher {
    roots: RecommendedWatcher,
    files: Option<RecommendedWatcher>,
    file_paths: HashSet<PathBuf>,
    handler: SharedHandler,
    config: Config,
}

fn backend(handler: &SharedHandler, config: Config) -> notify::Result<RecommendedWatcher> {
    let handler = Arc::clone(handler);
    RecommendedWatcher::new(
        move |event: notify::Result<Event>| {
            if matches!(&event, Ok(event) if is_read_only_access_event(event)) {
                return;
            }
            handler
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .handle_event(event);
        },
        config,
    )
}

impl Watcher for ContentChangeWatcher {
    fn new<F: EventHandler>(event_handler: F, config: Config) -> notify::Result<Self> {
        let handler: SharedHandler = Arc::new(Mutex::new(event_handler));
        Ok(Self {
            roots: backend(&handler, config)?,
            files: None,
            file_paths: HashSet::new(),
            handler,
            config,
        })
    }

    fn watch(&mut self, path: &Path, recursive_mode: RecursiveMode) -> notify::Result<()> {
        if recursive_mode == RecursiveMode::Recursive {
            return self.roots.watch(path, recursive_mode);
        }
        let files = match self.files.take() {
            Some(files) => files,
            None => backend(&self.handler, self.config)?,
        };
        self.files.insert(files).watch(path, recursive_mode)?;
        self.file_paths.insert(path.to_path_buf());
        Ok(())
    }

    fn unwatch(&mut self, path: &Path) -> notify::Result<()> {
        if !self.file_paths.remove(path) {
            return self.roots.unwatch(path);
        }
        match &mut self.files {
            Some(files) => files.unwatch(path),
            None => Ok(()),
        }
    }

    fn configure(&mut self, config: Config) -> notify::Result<bool> {
        self.config = config;
        if let Some(files) = &mut self.files {
            files.configure(config)?;
        }
        self.roots.configure(config)
    }

    fn kind() -> WatcherKind {
        RecommendedWatcher::kind()
    }
}
