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
}

pub(super) fn publish(output_dir: &Path, files: &[ComponentAssetFile]) -> Result<()> {
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
        let current = manifest_for(files);
        for file in files {
            write_staged_file(&temporary_dir, file)?;
        }
        let manifest =
            serde_json::to_vec(&current).context("Failed to serialize component assets")?;
        fs::write(temporary_dir.join(MANIFEST_FILE), manifest)
            .context("Failed to stage component asset manifest")?;

        let previous = read_manifest(output_dir)?;
        publish_group(output_dir, &temporary_dir, files, false)?;
        publish_group(output_dir, &temporary_dir, files, true)?;
        remove_stale(output_dir, &previous, &current)?;
        replace_file(
            &temporary_dir.join(MANIFEST_FILE),
            &output_dir.join(MANIFEST_FILE),
        )
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
    };
    for file in files {
        manifest.files.push(file.name.clone());
        if is_root_file(file) {
            manifest.roots.push(file.name.clone());
        }
    }
    manifest
}

fn publish_group(
    output_dir: &Path,
    temporary_dir: &Path,
    files: &[ComponentAssetFile],
    roots: bool,
) -> Result<()> {
    for file in files {
        if is_root_file(file) != roots {
            continue;
        }
        replace_file(
            &temporary_dir.join(&file.name),
            &output_dir.join(&file.name),
        )?;
    }
    Ok(())
}

fn is_root_file(file: &ComponentAssetFile) -> bool {
    !file.name.starts_with("components/")
}

fn remove_stale(
    output_dir: &Path,
    previous: &PublishedFiles,
    current: &PublishedFiles,
) -> Result<()> {
    let current_files: HashSet<&str> = current.files.iter().map(String::as_str).collect();
    let previous_roots: HashSet<&str> = previous.roots.iter().map(String::as_str).collect();
    for roots in [true, false] {
        for file in &previous.files {
            if current_files.contains(file.as_str())
                || previous_roots.contains(file.as_str()) != roots
                || !is_owned_file_name(file)
            {
                continue;
            }
            let path = output_dir.join(file);
            if path.is_file() {
                fs::remove_file(&path)
                    .with_context(|| format!("Failed to remove stale {}", path.display()))?;
            }
        }
    }
    Ok(())
}

fn is_owned_file_name(name: &str) -> bool {
    if name.contains('\\') {
        return false;
    }
    let mut parts = name.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(file), None, None) => is_component_asset_file_name(file),
        (Some("components"), Some(file), None) => is_component_asset_file_name(file),
        _ => false,
    }
}

fn is_component_asset_file_name(name: &str) -> bool {
    let Some(tag) = name.strip_suffix(".webui.js") else {
        return false;
    };
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

fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    if let Err(error) = fs::rename(source, destination) {
        if !destination.is_file() {
            return Err(error)
                .with_context(|| format!("Failed to publish {}", destination.display()));
        }
        fs::remove_file(destination)
            .with_context(|| format!("Failed to replace {}", destination.display()))?;
        fs::rename(source, destination)
            .with_context(|| format!("Failed to publish {}", destination.display()))?;
    }
    Ok(())
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
        let output = TempDir::new().unwrap();
        publish(
            output.path(),
            &[
                file("component-old-card.webui.js", "old component"),
                file("old-root.webui.js", "old root"),
            ],
        )
        .unwrap();
        fs::write(output.path().join("application.js"), "owned by app").unwrap();

        publish(
            output.path(),
            &[
                file("components/new-card.webui.js", "new component"),
                file("new-root.webui.js", "new root"),
            ],
        )
        .unwrap();

        assert!(!output.path().join("component-old-card.webui.js").exists());
        assert!(!output.path().join("old-root.webui.js").exists());
        assert!(output.path().join("components/new-card.webui.js").is_file());
        assert!(output.path().join("new-root.webui.js").is_file());
        let manifest = read_manifest(output.path()).unwrap();
        assert_eq!(manifest.roots, ["new-root.webui.js"]);
        assert_eq!(
            fs::read_to_string(output.path().join("application.js")).unwrap(),
            "owned by app"
        );
    }
}
