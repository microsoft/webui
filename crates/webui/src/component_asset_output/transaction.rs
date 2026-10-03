// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{
    is_owned_file_name, is_root_file, ComponentAssetFile, Context, PublishedFiles, Result,
    WebUIError,
};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

enum PublishedChange {
    Created(PathBuf),
    Replaced {
        destination: PathBuf,
        backup: PathBuf,
    },
}

pub(super) struct Publication<'a> {
    output_dir: &'a Path,
    temporary_dir: &'a Path,
    changes: Vec<PublishedChange>,
}

impl<'a> Publication<'a> {
    pub(super) fn new(output_dir: &'a Path, temporary_dir: &'a Path) -> Self {
        Self {
            output_dir,
            temporary_dir,
            changes: Vec::new(),
        }
    }

    pub(super) fn publish_group(
        &mut self,
        files: &[&ComponentAssetFile],
        roots: bool,
    ) -> Result<()> {
        for file in files {
            if is_root_file(file) == roots {
                self.replace(&file.name)?;
            }
        }
        Ok(())
    }

    pub(super) fn remove_stale(
        &mut self,
        previous: &PublishedFiles,
        current: &PublishedFiles,
    ) -> Result<()> {
        let current_roots: HashSet<&str> = current.roots.iter().map(String::as_str).collect();
        for file in &previous.roots {
            if current_roots.contains(file.as_str())
                || !is_owned_file_name(file)
                || file.starts_with("components/")
            {
                continue;
            }
            let destination = self.output_dir.join(file);
            if destination.is_file() {
                self.back_up(file, &destination)?;
                fs::remove_file(&destination).context("Failed to remove stale component root")?;
            }
        }
        Ok(())
    }

    pub(super) fn replace(&mut self, relative: &str) -> Result<()> {
        let source = self.temporary_dir.join(relative);
        let destination = self.output_dir.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        if destination.exists() {
            if !destination.is_file() {
                return Err(WebUIError::InvalidBuildOptions(format!(
                    "Cannot publish component asset over non-file {}",
                    destination.display()
                )));
            }
            self.back_up(relative, &destination)?;
        } else if !relative.starts_with("components/") {
            self.changes
                .push(PublishedChange::Created(destination.clone()));
        }
        fs::rename(&source, &destination)
            .with_context(|| format!("Failed to publish {}", destination.display()))
    }

    fn back_up(&mut self, relative: &str, destination: &Path) -> Result<()> {
        let backup = self.temporary_dir.join("backup").join(relative);
        if let Some(parent) = backup.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        fs::copy(destination, &backup)
            .with_context(|| format!("Failed to preserve {}", destination.display()))?;
        self.changes.push(PublishedChange::Replaced {
            destination: destination.to_path_buf(),
            backup,
        });
        Ok(())
    }

    pub(super) fn rollback(&mut self) -> Result<()> {
        let mut failure = None;
        while let Some(change) = self.changes.pop() {
            let result = match change {
                PublishedChange::Created(destination) => remove_if_file(&destination),
                PublishedChange::Replaced {
                    destination,
                    backup,
                } => fs::rename(backup, &destination)
                    .with_context(|| format!("Failed to restore {}", destination.display())),
            };
            if failure.is_none() {
                failure = result.err();
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

fn remove_if_file(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("Failed to roll back {}", path.display())),
    }
}
