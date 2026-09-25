// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::paths::Destination;
use crate::{
    error::{io, schema},
    GenerateError,
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
#[path = "transaction_tests.rs"]
mod tests;

pub(super) struct Change {
    pub destination: Destination,
    pub before: Option<Vec<u8>>,
    pub after: Option<Vec<u8>>,
    pub order: u8,
}

struct Staged {
    change: Change,
    directory: PathBuf,
    backed_up: bool,
    installed: bool,
}

pub(super) fn publish(changes: Vec<Change>) -> Result<(), GenerateError> {
    publish_with(changes, |_| Ok(()))
}

fn publish_with(
    mut changes: Vec<Change>,
    mut after_install: impl FnMut(&Path) -> std::io::Result<()>,
) -> Result<(), GenerateError> {
    changes.sort_by_key(|change| change.order);
    let mut staged = Vec::with_capacity(changes.len());
    let mut directories = Vec::new();
    let preparation = prepare(changes, &mut staged, &mut directories);
    if let Err(error) = preparation {
        return finish_failure(error, &mut staged, &directories);
    }
    for index in 0..staged.len() {
        let result = install(&mut staged[index]).and_then(|()| {
            let path = &staged[index].change.destination.path;
            after_install(path).map_err(|e| io(path, e))
        });
        if let Err(error) = result {
            return finish_failure(error, &mut staged, &directories);
        }
    }
    // Backups remain available until every payload, removal, lock and inventory succeeds.
    cleanup(&staged, &[]).map_err(|error| schema(
        "ipc-publication-cleanup", "generated outputs",
        format!("all artifacts were published, but staging cleanup failed: {error}"),
        "outputs, lock and inventory are current; remove the remaining .webui-ipc-publish-* staging directories",
    ))?;
    Ok(())
}

fn prepare(
    changes: Vec<Change>,
    staged: &mut Vec<Staged>,
    directories: &mut Vec<PathBuf>,
) -> Result<(), GenerateError> {
    for change in changes {
        change.destination.validate()?;
        let path = &change.destination.path;
        let parent = path.parent().ok_or_else(|| {
            schema(
                "ipc-output-path",
                path.display().to_string(),
                "output has no parent",
                "choose a file inside an output directory",
            )
        })?;
        create_parents(parent, directories)?;
        change.destination.validate()?;
        let directory = parent.join(format!(
            ".webui-ipc-publish-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).map_err(|e| io(&directory, e))?;
        staged.push(Staged {
            change,
            directory,
            backed_up: false,
            installed: false,
        });
        let entry = &staged[staged.len() - 1];
        if let Some(bytes) = &entry.change.after {
            let new = entry.directory.join("new");
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&new)
                .map_err(|e| io(&new, e))?;
            file.write_all(bytes).map_err(|e| io(&new, e))?;
            if entry.change.before.is_some() {
                let permissions = fs::metadata(&entry.change.destination.path)
                    .map_err(|e| io(&entry.change.destination.path, e))?
                    .permissions();
                file.set_permissions(permissions).map_err(|e| io(&new, e))?;
            }
            file.sync_all().map_err(|e| io(&new, e))?;
        }
    }
    Ok(())
}

fn install(entry: &mut Staged) -> Result<(), GenerateError> {
    entry.change.destination.validate()?;
    let destination = &entry.change.destination.path;
    let actual = match fs::read(destination) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(io(destination, e)),
    };
    if actual != entry.change.before {
        return Err(schema(
            "ipc-output-changed",
            destination.display().to_string(),
            "artifact changed during generation",
            "stop concurrent writers and retry generation",
        ));
    }
    if actual.is_some() {
        fs::rename(destination, entry.directory.join("old")).map_err(|e| io(destination, e))?;
        entry.backed_up = true;
    }
    if entry.change.after.is_some() {
        fs::rename(entry.directory.join("new"), destination).map_err(|e| io(destination, e))?;
        entry.installed = true;
    }
    Ok(())
}

fn finish_failure(
    error: GenerateError,
    staged: &mut [Staged],
    directories: &[PathBuf],
) -> Result<(), GenerateError> {
    let mut failures = Vec::new();
    for entry in staged.iter_mut().rev() {
        if let Err(e) = restore(entry) {
            failures.push(e.to_string());
        }
    }
    if !failures.is_empty() {
        return Err(schema("ipc-publication-rollback", "generated outputs", format!("{error}; rollback errors: {}", failures.join("; ")),
            "stop concurrent writers; restore originals from retained .webui-ipc-publish-*/old backups before retrying"));
    }
    if let Err(cleanup_error) = cleanup(staged, directories) {
        return Err(schema("ipc-publication-cleanup", "generated outputs", format!("{error}; cleanup error: {cleanup_error}"),
            "original artifacts were restored; remove remaining .webui-ipc-publish-* staging directories before retrying"));
    }
    Err(error)
}

fn restore(entry: &mut Staged) -> Result<(), GenerateError> {
    if !entry.installed && !entry.backed_up {
        return Ok(());
    }
    entry.change.destination.validate()?;
    let path = &entry.change.destination.path;
    if entry.installed {
        fs::remove_file(path).map_err(|e| io(path, e))?;
        entry.installed = false;
    }
    if entry.backed_up {
        fs::rename(entry.directory.join("old"), path).map_err(|e| io(path, e))?;
        entry.backed_up = false;
    }
    Ok(())
}

fn create_parents(parent: &Path, created: &mut Vec<PathBuf>) -> Result<(), GenerateError> {
    let mut missing = Vec::new();
    let mut next = parent;
    while !next.exists() {
        missing.push(next.to_owned());
        next = next.parent().ok_or_else(|| {
            io(
                next,
                std::io::Error::other("output has no existing ancestor"),
            )
        })?;
    }
    for directory in missing.into_iter().rev() {
        fs::create_dir(&directory).map_err(|e| io(&directory, e))?;
        created.push(directory);
    }
    Ok(())
}

fn cleanup(staged: &[Staged], directories: &[PathBuf]) -> Result<(), GenerateError> {
    for entry in staged {
        for name in ["new", "old"] {
            let path = entry.directory.join(name);
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(io(&path, e)),
            }
        }
        fs::remove_dir(&entry.directory).map_err(|e| io(&entry.directory, e))?;
    }
    for directory in directories.iter().rev() {
        fs::remove_dir(directory).map_err(|e| io(directory, e))?;
    }
    Ok(())
}
