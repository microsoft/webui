// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::{
    error::{io, schema},
    GenerateError,
};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Clone)]
pub(super) struct Scope {
    logical: PathBuf,
    anchor: PathBuf,
    root: PathBuf,
}

#[derive(Clone)]
pub(super) struct Destination {
    scope: Scope,
    pub path: PathBuf,
}

impl Scope {
    pub fn new(root: &Path) -> Result<Self, GenerateError> {
        let logical = absolute(root)?;
        let mut ancestor = logical.as_path();
        loop {
            match fs::symlink_metadata(ancestor) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink() || !metadata.is_dir() {
                        return Err(invalid(
                            ancestor,
                            "output root or nearest existing parent is not a real directory",
                        ));
                    }
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    ancestor = ancestor
                        .parent()
                        .ok_or_else(|| invalid(root, "output has no existing parent"))?;
                }
                Err(e) => return Err(io(ancestor, e)),
            }
        }
        let anchor = fs::canonicalize(ancestor).map_err(|e| io(ancestor, e))?;
        let suffix = logical
            .strip_prefix(ancestor)
            .map_err(|_| invalid(root, "invalid output root"))?;
        let resolved = anchor.join(suffix);
        Ok(Self {
            logical,
            anchor,
            root: resolved,
        })
    }

    pub fn destination(&self, path: &Path) -> Result<Destination, GenerateError> {
        let absolute = absolute(path)?;
        let relative = absolute
            .strip_prefix(&self.logical)
            .map_err(|_| invalid(path, "artifact escapes its configured output directory"))?;
        if relative.as_os_str().is_empty() {
            return Err(invalid(
                path,
                "artifact path is the output directory itself",
            ));
        }
        let destination = Destination {
            scope: self.clone(),
            path: self.root.join(relative),
        };
        destination.validate()?;
        Ok(destination)
    }
}

impl Destination {
    pub fn validate(&self) -> Result<(), GenerateError> {
        if !self.path.starts_with(&self.scope.root) {
            return Err(invalid(
                &self.path,
                "artifact escapes its canonical output root",
            ));
        }
        let mut current = self.scope.anchor.clone();
        check_component(&current, false)?;
        let relative = self
            .path
            .strip_prefix(&current)
            .map_err(|_| invalid(&self.path, "artifact escapes its existing parent"))?;
        for component in relative.components() {
            current.push(component);
            check_component(&current, current == self.path)?;
        }
        Ok(())
    }
}

fn check_component(path: &Path, leaf: bool) -> Result<(), GenerateError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(invalid(
                    path,
                    "symlinks are forbidden in artifact destinations and their output parents",
                ));
            }
            if (leaf && !metadata.is_file()) || (!leaf && !metadata.is_dir()) {
                return Err(invalid(
                    path,
                    "artifact must be a regular file beneath real directories",
                ));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(io(path, e)),
    }
    Ok(())
}

fn absolute(path: &Path) -> Result<PathBuf, GenerateError> {
    let full = std::path::absolute(path).map_err(|e| io(path, e))?;
    let mut normalized = PathBuf::new();
    for component in full.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            other => normalized.push(other),
        }
    }
    Ok(normalized)
}

fn invalid(path: &Path, message: &str) -> GenerateError {
    schema("ipc-output-path", path.display().to_string(), message,
        "use regular generated files and directories, remove output symlinks, and keep artifacts inside their configured roots")
}
