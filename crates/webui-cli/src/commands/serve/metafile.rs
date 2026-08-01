// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use anyhow::{Context, Result};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static WRITE_ID: AtomicU64 = AtomicU64::new(0);
const TEMP_DIRECTORY: &str = ".webui-metafile-tmp";

pub(super) fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Metafile path must name a file: {}", path.display()))?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    let temporary_directory = temp_directory(path);
    fs::create_dir_all(&temporary_directory)
        .with_context(|| format!("Failed to create {}", temporary_directory.display()))?;
    let write_id = WRITE_ID.fetch_add(1, Ordering::Relaxed);
    let mut temporary_name = OsString::new();
    temporary_name.push(file_name);
    temporary_name.push(format!(".{}.{}.tmp", std::process::id(), write_id));
    let temporary_path = temporary_directory.join(temporary_name);
    if let Err(error) = fs::write(&temporary_path, content) {
        let _ = fs::remove_file(&temporary_path);
        let _ = fs::remove_dir(&temporary_directory);
        return Err(error).with_context(|| format!("Failed to write {}", temporary_path.display()));
    }
    if let Err(error) = fs::rename(&temporary_path, path) {
        let _ = fs::remove_file(&temporary_path);
        let _ = fs::remove_dir(&temporary_directory);
        return Err(error).with_context(|| format!("Failed to replace {}", path.display()));
    }
    let _ = fs::remove_dir(&temporary_directory);
    Ok(())
}

pub(super) fn watch_ignore_paths(path: &Path) -> [PathBuf; 2] {
    let parent = existing_parent(path);
    let metafile = fs::canonicalize(path).unwrap_or_else(|_| {
        path.file_name()
            .map_or_else(|| path.to_path_buf(), |name| parent.join(name))
    });
    [metafile, parent.join(TEMP_DIRECTORY)]
}

pub(super) fn temp_directory(path: &Path) -> PathBuf {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .join(TEMP_DIRECTORY)
}

fn existing_parent(path: &Path) -> PathBuf {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[cfg(unix)]
    #[test]
    fn watch_ignore_paths_resolve_a_symlinked_metafile_parent() {
        use std::os::unix::fs::symlink;

        let root = TempDir::new().unwrap();
        let real_parent = root.path().join("real");
        let linked_parent = root.path().join("linked");
        fs::create_dir(&real_parent).unwrap();
        symlink(&real_parent, &linked_parent).unwrap();
        let metafile = linked_parent.join("component-assets.meta.json");
        write_atomic(&metafile, "{}").unwrap();

        let paths = watch_ignore_paths(&metafile);
        let canonical_parent = fs::canonicalize(&real_parent).unwrap();
        assert_eq!(
            paths,
            [
                canonical_parent.join("component-assets.meta.json"),
                canonical_parent.join(TEMP_DIRECTORY),
            ]
        );
    }
}
