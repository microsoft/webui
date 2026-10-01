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
            fs::write(temporary_dir.join(&file.name), &file.content)
                .with_context(|| format!("Failed to stage component asset {}", file.name))?;
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

fn manifest_for(files: &[ComponentAssetFile]) -> PublishedFiles {
    let mut manifest = PublishedFiles {
        files: Vec::with_capacity(files.len()),
        roots: Vec::new(),
    };
    for file in files {
        manifest.files.push(file.name.clone());
        if file.content.contains("__webuiDefineComponentAsset") {
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
        if file.content.contains("__webuiDefineComponentAsset") != roots {
            continue;
        }
        replace_file(
            &temporary_dir.join(&file.name),
            &output_dir.join(&file.name),
        )?;
    }
    Ok(())
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
    Path::new(name)
        .file_name()
        .is_some_and(|file_name| file_name == name)
        && name.ends_with(".webui.js")
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

    fn file(name: &str, root: bool, content: &str) -> ComponentAssetFile {
        ComponentAssetFile {
            name: name.to_string(),
            content: if root {
                format!("const __webuiDefineComponentAsset=1;{content}")
            } else {
                content.to_string()
            },
        }
    }

    #[test]
    fn publish_replaces_complete_generation_and_removes_stale_files() {
        let output = TempDir::new().unwrap();
        publish(
            output.path(),
            &[
                file("component-old.webui.js", false, "old component"),
                file("old.webui.js", true, "old root"),
            ],
        )
        .unwrap();
        fs::write(output.path().join("application.js"), "owned by app").unwrap();

        publish(
            output.path(),
            &[
                file("component-new.webui.js", false, "new component"),
                file("new.webui.js", true, "new root"),
            ],
        )
        .unwrap();

        assert!(!output.path().join("component-old.webui.js").exists());
        assert!(!output.path().join("old.webui.js").exists());
        assert!(output.path().join("component-new.webui.js").is_file());
        assert!(output.path().join("new.webui.js").is_file());
        assert_eq!(
            fs::read_to_string(output.path().join("application.js")).unwrap(),
            "owned by app"
        );
    }
}
