// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{is_root_file, ComponentAssetFile, Context, Result, WebUIError};
use std::fs;
use std::path::{Path, PathBuf};

enum PublishedChange {
    CreatedRoot(PathBuf),
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
                if roots {
                    self.replace_root(&file.name)?;
                } else {
                    self.publish_payload(&file.name)?;
                }
            }
        }
        Ok(())
    }

    fn publish_payload(&self, relative: &str) -> Result<()> {
        let source = self.temporary_dir.join(relative);
        let destination = self.output_dir.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        match fs::hard_link(&source, &destination) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if fs::read(&source).with_context(|| {
                    format!("Failed to read staged component asset {}", source.display())
                })? == fs::read(&destination).with_context(|| {
                    format!(
                        "Failed to read published component asset {}",
                        destination.display()
                    )
                })? {
                    Ok(())
                } else {
                    Err(WebUIError::InvalidBuildOptions(format!(
                        "Component asset content hash collision for {}",
                        destination.display()
                    )))
                }
            }
            Err(error) => {
                Err(error).with_context(|| format!("Failed to publish {}", destination.display()))
            }
        }
    }

    fn replace_root(&mut self, relative: &str) -> Result<()> {
        let source = self.temporary_dir.join(relative);
        let destination = self.output_dir.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        match fs::symlink_metadata(&destination) {
            Ok(metadata) => {
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err(WebUIError::InvalidBuildOptions(format!(
                        "Cannot publish component asset over non-file or symlink {}",
                        destination.display()
                    )));
                }
                self.back_up(relative, &destination)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.changes
                    .push(PublishedChange::CreatedRoot(destination.clone()));
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("Failed to inspect {}", destination.display()));
            }
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
                PublishedChange::CreatedRoot(destination) => remove_if_file(&destination),
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
