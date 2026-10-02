// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use webui::ComponentAssetFile;

const MANIFEST_FILE: &str = ".webui-component-assets.json";
static PUBLISH_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Default, Deserialize, Serialize)]
struct PublishedFiles {
    files: Vec<String>,
    roots: Vec<String>,
    #[serde(default)]
    retired: Vec<String>,
}

#[cfg(test)]
pub(super) fn publish(output_dir: &Path, files: &[ComponentAssetFile]) -> Result<()> {
    publish_with(output_dir, files, || Ok(()))
}

pub(super) fn publish_with<F>(
    output_dir: &Path,
    files: &[ComponentAssetFile],
    finish: F,
) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    fs::create_dir_all(output_dir)
        .with_context(|| format!("Failed to create {}", output_dir.display()))?;
    let publish_id = PUBLISH_ID.fetch_add(1, Ordering::Relaxed);
    let temporary_dir = output_dir.join(format!(
        ".webui-component-assets-tmp-{}-{publish_id}",
        std::process::id()
    ));
    fs::create_dir(&temporary_dir)
        .with_context(|| format!("Failed to create {}", temporary_dir.display()))?;

    let result = (|| {
        let mut current = manifest_for(files);
        for file in files {
            write_staged_file(&temporary_dir, file)?;
        }

        let previous = read_manifest(output_dir)?;
        let current_files: HashSet<&str> = current.files.iter().map(String::as_str).collect();
        current.retired = previous
            .files
            .iter()
            .filter(|file| {
                file.starts_with("components/") && !current_files.contains(file.as_str())
            })
            .cloned()
            .collect();
        let manifest =
            serde_json::to_vec(&current).context("Failed to serialize component assets")?;
        fs::write(temporary_dir.join(MANIFEST_FILE), manifest)
            .context("Failed to stage component asset manifest")?;

        let mut publication = Publication::new(output_dir, &temporary_dir);
        let published = (|| {
            publication.remove_retired(&previous)?;
            publication.publish_group(files, false)?;
            publication.publish_group(files, true)?;
            publication.remove_stale(&previous, &current)?;
            publication.replace(MANIFEST_FILE)?;
            finish()
        })();
        match published {
            Ok(()) => Ok(()),
            Err(error) => match publication.rollback() {
                Ok(()) => Err(error),
                Err(rollback) => Err(error.context(format!("Rollback also failed: {rollback:#}"))),
            },
        }
    })();
    let _ = fs::remove_dir_all(&temporary_dir);
    result
}

pub(super) fn watch_ignore_paths(output_dir: &Path) -> [PathBuf; 1] {
    [output_dir.to_path_buf()]
}

pub(super) fn manifest_path(output_dir: &Path) -> PathBuf {
    output_dir.join(MANIFEST_FILE)
}

fn manifest_for(files: &[ComponentAssetFile]) -> PublishedFiles {
    let mut manifest = PublishedFiles {
        files: Vec::with_capacity(files.len()),
        roots: Vec::new(),
        retired: Vec::new(),
    };
    for file in files {
        manifest.files.push(file.name.clone());
        if is_root_file(file) {
            manifest.roots.push(file.name.clone());
        }
    }
    manifest
}

fn is_root_file(file: &ComponentAssetFile) -> bool {
    !file.name.starts_with("components/")
}

fn is_owned_file_name(name: &str) -> bool {
    if name.contains('\\') {
        return false;
    }
    let mut parts = name.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(file), None, None) => is_component_asset_file_name(file),
        (Some("components"), Some(file), None) => is_component_payload_file_name(file),
        _ => false,
    }
}

fn is_component_payload_file_name(name: &str) -> bool {
    const CONTENT_ID_LEN: usize = 16;
    let Some(stem) = name.strip_suffix(".webui.js") else {
        return false;
    };
    let Some((tag, content_id)) = stem.rsplit_once('.') else {
        return false;
    };
    content_id.len() == CONTENT_ID_LEN
        && content_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && is_component_tag(tag)
}

fn is_component_asset_file_name(name: &str) -> bool {
    let Some(tag) = name.strip_suffix(".webui.js") else {
        return false;
    };
    is_component_tag(tag)
}

fn is_component_tag(tag: &str) -> bool {
    let bytes = tag.as_bytes();
    !bytes.is_empty()
        && bytes.contains(&b'-')
        && bytes[0].is_ascii_lowercase()
        && bytes[bytes.len() - 1].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn write_staged_file(temporary_dir: &Path, file: &ComponentAssetFile) -> Result<()> {
    let path = temporary_dir.join(&file.name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    fs::write(path, &file.content)
        .with_context(|| format!("Failed to stage component asset {}", file.name))
}

fn read_manifest(output_dir: &Path) -> Result<PublishedFiles> {
    let path = output_dir.join(MANIFEST_FILE);
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("Failed to parse {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(PublishedFiles::default()),
        Err(error) => Err(error).with_context(|| format!("Failed to read {}", path.display())),
    }
}

enum PublishedChange {
    Created(PathBuf),
    Replaced {
        destination: PathBuf,
        backup: PathBuf,
    },
}

struct Publication<'a> {
    output_dir: &'a Path,
    temporary_dir: &'a Path,
    changes: Vec<PublishedChange>,
}

impl<'a> Publication<'a> {
    fn new(output_dir: &'a Path, temporary_dir: &'a Path) -> Self {
        Self {
            output_dir,
            temporary_dir,
            changes: Vec::new(),
        }
    }

    fn publish_group(&mut self, files: &[ComponentAssetFile], roots: bool) -> Result<()> {
        for file in files {
            if is_root_file(file) == roots {
                self.replace(&file.name)?;
            }
        }
        Ok(())
    }

    fn remove_stale(&mut self, previous: &PublishedFiles, current: &PublishedFiles) -> Result<()> {
        let current_files: HashSet<&str> = current.files.iter().map(String::as_str).collect();
        let previous_roots: HashSet<&str> = previous.roots.iter().map(String::as_str).collect();
        for roots in [true, false] {
            for file in &previous.files {
                if current_files.contains(file.as_str())
                    || previous_roots.contains(file.as_str()) != roots
                    || !is_owned_file_name(file)
                    || file.starts_with("components/")
                {
                    continue;
                }
                let destination = self.output_dir.join(file);
                if destination.is_file() {
                    self.back_up(file, &destination)?;
                }
            }
        }
        Ok(())
    }

    fn remove_retired(&mut self, previous: &PublishedFiles) -> Result<()> {
        for file in &previous.retired {
            if !file.starts_with("components/") || !is_owned_file_name(file) {
                continue;
            }
            let destination = self.output_dir.join(file);
            if destination.is_file() {
                self.back_up(file, &destination)?;
            }
        }
        Ok(())
    }

    fn replace(&mut self, relative: &str) -> Result<()> {
        let source = self.temporary_dir.join(relative);
        let destination = self.output_dir.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        if destination.exists() {
            if !destination.is_file() {
                anyhow::bail!(
                    "Cannot publish component asset over non-file {}",
                    destination.display()
                );
            }
            if relative.starts_with("components/") {
                let source_content = fs::read(&source).with_context(|| {
                    format!("Failed to read staged component asset {}", source.display())
                })?;
                let destination_content = fs::read(&destination).with_context(|| {
                    format!(
                        "Failed to read published component asset {}",
                        destination.display()
                    )
                })?;
                if source_content == destination_content {
                    fs::remove_file(source).with_context(|| {
                        format!("Failed to discard unchanged component asset {}", relative)
                    })?;
                    return Ok(());
                }
                anyhow::bail!("Component asset content hash collision for {relative}");
            }
            self.back_up(relative, &destination)?;
        } else {
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
        fs::rename(destination, &backup)
            .with_context(|| format!("Failed to preserve {}", destination.display()))?;
        self.changes.push(PublishedChange::Replaced {
            destination: destination.to_path_buf(),
            backup,
        });
        Ok(())
    }

    fn rollback(&mut self) -> Result<()> {
        let mut failure = None;
        while let Some(change) = self.changes.pop() {
            let result = match change {
                PublishedChange::Created(destination) => remove_if_file(&destination),
                PublishedChange::Replaced {
                    destination,
                    backup,
                } => restore_backup(&destination, &backup),
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

fn restore_backup(destination: &Path, backup: &Path) -> Result<()> {
    remove_if_file(destination)?;
    fs::rename(backup, destination)
        .with_context(|| format!("Failed to restore {}", destination.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn file(name: &str, content: &str) -> ComponentAssetFile {
        ComponentAssetFile {
            name: name.to_string(),
            content: content.to_string(),
        }
    }

    #[test]
    fn publish_replaces_complete_generation_and_removes_stale_files() {
        const OLD_PAYLOAD: &str = "components/old-card.aaaaaaaaaaaaaaaa.webui.js";
        const NEW_PAYLOAD: &str = "components/new-card.bbbbbbbbbbbbbbbb.webui.js";
        const NEXT_PAYLOAD: &str = "components/next-card.cccccccccccccccc.webui.js";
        let output = TempDir::new().unwrap();
        publish(
            output.path(),
            &[
                file(OLD_PAYLOAD, "old component"),
                file("old-root.webui.js", "old root"),
            ],
        )
        .unwrap();
        fs::write(output.path().join("application.js"), "owned by app").unwrap();

        publish(
            output.path(),
            &[
                file(NEW_PAYLOAD, "new component"),
                file("new-root.webui.js", "new root"),
            ],
        )
        .unwrap();

        assert!(!output.path().join("old-root.webui.js").exists());
        assert!(output.path().join(OLD_PAYLOAD).is_file());
        assert!(output.path().join(NEW_PAYLOAD).is_file());
        assert!(output.path().join("new-root.webui.js").is_file());
        let manifest = read_manifest(output.path()).unwrap();
        assert_eq!(manifest.roots, ["new-root.webui.js"]);
        assert_eq!(manifest.retired, [OLD_PAYLOAD]);

        publish(
            output.path(),
            &[
                file(NEXT_PAYLOAD, "next component"),
                file("next-root.webui.js", "next root"),
            ],
        )
        .unwrap();
        assert!(!output.path().join(OLD_PAYLOAD).exists());
        assert!(output.path().join(NEW_PAYLOAD).is_file());
        assert!(output.path().join(NEXT_PAYLOAD).is_file());
        assert_eq!(
            fs::read_to_string(output.path().join("application.js")).unwrap(),
            "owned by app"
        );
    }

    #[test]
    fn publication_rolls_back_replacements_after_late_failure() {
        let output = TempDir::new().unwrap();
        let staging = TempDir::new_in(output.path()).unwrap();
        fs::write(output.path().join("first.webui.js"), "old first").unwrap();
        fs::write(staging.path().join("first.webui.js"), "new first").unwrap();

        let mut publication = Publication::new(output.path(), staging.path());
        publication.replace("first.webui.js").unwrap();
        assert!(publication.replace("missing.webui.js").is_err());
        publication.rollback().unwrap();

        assert_eq!(
            fs::read_to_string(output.path().join("first.webui.js")).unwrap(),
            "old first"
        );
        assert!(!output.path().join("missing.webui.js").exists());
    }

    #[test]
    fn finish_failure_rolls_back_asset_generation() {
        let output = TempDir::new().unwrap();
        publish(output.path(), &[file("stable-root.webui.js", "old root")]).unwrap();

        let error = publish_with(
            output.path(),
            &[file("stable-root.webui.js", "new root")],
            || anyhow::bail!("metafile commit failed"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("metafile commit failed"));
        assert_eq!(
            fs::read_to_string(output.path().join("stable-root.webui.js")).unwrap(),
            "old root"
        );
    }

    #[test]
    fn content_hash_collision_preserves_published_payload() {
        const PAYLOAD: &str = "components/test-card.0000000100000000.webui.js";
        let output = TempDir::new().unwrap();
        publish(output.path(), &[file(PAYLOAD, "a")]).unwrap();

        let error = publish(output.path(), &[file(PAYLOAD, "b")]).unwrap_err();

        assert!(error.to_string().contains("content hash collision"));
        assert_eq!(
            fs::read_to_string(output.path().join(PAYLOAD)).unwrap(),
            "a"
        );
    }
}
