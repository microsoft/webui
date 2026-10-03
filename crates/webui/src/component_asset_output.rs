// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Transactional publication of generated component ESM inputs.

#[path = "component_asset_output/transaction.rs"]
mod transaction;

use crate::{ComponentAssetFile, WebUIError};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use transaction::Publication;

type Result<T> = std::result::Result<T, WebUIError>;

trait Context<T> {
    fn context(self, context: impl Into<String>) -> Result<T>;
    fn with_context(self, context: impl FnOnce() -> String) -> Result<T>;
}

impl<T, E: std::error::Error + Send + Sync + 'static> Context<T> for std::result::Result<T, E> {
    fn context(self, context: impl Into<String>) -> Result<T> {
        self.map_err(|source| publication_error(context.into(), source))
    }

    fn with_context(self, context: impl FnOnce() -> String) -> Result<T> {
        self.map_err(|source| publication_error(context(), source))
    }
}

#[cold]
#[inline(never)]
fn publication_error(
    context: String,
    source: impl std::error::Error + Send + Sync + 'static,
) -> WebUIError {
    WebUIError::ComponentAssetPublication {
        context,
        source: Box::new(source),
    }
}

const MANIFEST_FILE: &str = ".webui-component-assets.json";
static PUBLISH_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Default, Deserialize, Serialize)]
struct PublishedFiles {
    files: Vec<String>,
    roots: Vec<String>,
}

/// Publish immutable dependencies before switching stable root entries.
///
/// Unchanged files keep their identity and modification times. Old immutable
/// payloads remain available until [`prune`] is called with readers quiescent.
/// Publishers targeting one directory must be serialized by the host.
///
/// # Errors
///
/// Returns an error on invalid filenames, content-address collisions, or I/O
/// failure. Failed publication restores stable roots and bookkeeping. Immutable
/// payloads stay available even if a reader observed a root before rollback.
#[must_use = "component asset publication failures must be handled"]
pub fn publish(output_dir: &Path, files: &[ComponentAssetFile]) -> Result<()> {
    publish_with(output_dir, files, || Ok(()))
}

/// Publish assets and finalize a related output within the rollback boundary.
///
/// `finish` must itself preserve its previous output on failure. It is invoked
/// even for an unchanged graph, without rewriting the component inputs.
///
/// # Errors
///
/// Returns publication or finalization errors after rolling back asset changes.
#[must_use = "component asset finalization failures must be handled"]
pub fn publish_with<F>(output_dir: &Path, files: &[ComponentAssetFile], finish: F) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    let current = manifest_for(files);
    validate_files(files)?;
    validate_payload_directory(output_dir)?;
    fs::create_dir_all(output_dir)
        .with_context(|| format!("Failed to create {}", output_dir.display()))?;
    let previous = read_manifest(output_dir)?;
    let mut changed = Vec::with_capacity(files.len());
    for file in files {
        if !unchanged_file(output_dir, file)? {
            changed.push(file);
        }
    }
    let same_manifest = current.files == previous.files && current.roots == previous.roots;
    if changed.is_empty() && same_manifest {
        return finish();
    }
    let publish_id = PUBLISH_ID.fetch_add(1, Ordering::Relaxed);
    let temporary_dir = output_dir.join(format!(
        ".webui-component-assets-tmp-{}-{publish_id}",
        std::process::id()
    ));
    fs::create_dir(&temporary_dir)
        .with_context(|| format!("Failed to create {}", temporary_dir.display()))?;

    let mut preserve_staging = false;
    let result = (|| {
        for file in &changed {
            write_staged_file(&temporary_dir, file)?;
        }

        if !same_manifest {
            let manifest =
                serde_json::to_vec(&current).context("Failed to serialize component assets")?;
            fs::write(temporary_dir.join(MANIFEST_FILE), manifest)
                .context("Failed to stage component asset manifest")?;
        }

        let mut publication = Publication::new(output_dir, &temporary_dir);
        let published = (|| {
            publication.publish_group(&changed, false)?;
            publication.publish_group(&changed, true)?;
            publication.remove_stale(&previous, &current)?;
            if !same_manifest {
                publication.replace(MANIFEST_FILE)?;
            }
            finish()
        })();
        match published {
            Ok(()) => Ok(()),
            Err(error) => match publication.rollback() {
                Ok(()) => Err(error),
                Err(rollback) => {
                    preserve_staging = true;
                    Err(WebUIError::ComponentAssetPublication {
                        context: format!(
                            "Rollback also failed: {}. Recovery files remain in {}",
                            rollback.chain_message(),
                            temporary_dir.display()
                        ),
                        source: Box::new(error),
                    })
                }
            },
        }
    })();
    if !preserve_staging {
        if let Err(error) = fs::remove_dir_all(&temporary_dir) {
            log::warn!(
                "Failed to clean component asset staging directory {}: {error}",
                temporary_dir.display()
            );
        }
    }
    result
}

/// The reserved bookkeeping path inside a component input directory.
#[must_use]
pub fn manifest_path(output_dir: &Path) -> PathBuf {
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

fn validate_files(files: &[ComponentAssetFile]) -> Result<()> {
    let mut seen = HashSet::with_capacity(files.len());
    for file in files {
        if !is_owned_file_name(&file.name) || !seen.insert(file.name.as_str()) {
            return Err(WebUIError::InvalidBuildOptions(format!(
                "invalid or duplicate generated component asset path '{}'",
                file.name
            )));
        }
    }
    Ok(())
}

fn unchanged_file(output_dir: &Path, file: &ComponentAssetFile) -> Result<bool> {
    let path = output_dir.join(&file.name);
    match fs::read(&path) {
        Ok(content) if content == file.content.as_bytes() => Ok(true),
        Ok(_) if file.name.starts_with("components/") => Err(WebUIError::InvalidBuildOptions(
            format!("Component asset content hash collision for {}", file.name),
        )),
        Ok(_) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => {
            Err(error).with_context(|| format!("Failed to read component asset {}", path.display()))
        }
    }
}

/// Remove obsolete immutable payloads after all readers of older roots finish.
///
/// The caller must stop or coordinate bundler/HTTP readers and publishers.
/// Publication never assumes that elapsed time or a generation count proves
/// readers are done. Only compiler-owned payload paths are removed.
///
/// # Errors
///
/// Returns an error if bookkeeping or payload files cannot be read or removed.
#[must_use = "component asset cleanup failures must be handled"]
pub fn prune(output_dir: &Path) -> Result<()> {
    validate_payload_directory(output_dir)?;
    let current = read_manifest(output_dir)?;
    let live: HashSet<&str> = current.files.iter().map(String::as_str).collect();
    let components = output_dir.join("components");
    let entries = match fs::read_dir(&components) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).context("Failed to read component payload directory"),
    };
    for entry in entries {
        let entry = entry.context("Failed to read component payload entry")?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let relative = format!("components/{name}");
        if is_owned_file_name(&relative)
            && !live.contains(relative.as_str())
            && entry
                .file_type()
                .context("Failed to inspect component payload")?
                .is_file()
        {
            fs::remove_file(entry.path()).context("Failed to prune obsolete component payload")?;
        }
    }
    Ok(())
}

fn validate_payload_directory(output_dir: &Path) -> Result<()> {
    let path = output_dir.join("components");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(WebUIError::InvalidBuildOptions(format!(
            "{} must be a real component payload directory. Remove the conflicting file or symlink, or choose another output directory.",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("Failed to inspect component payload directory"),
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

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
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
    fn publish_retains_arbitrarily_old_payloads_until_quiescent_pruning() {
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
        assert_eq!(
            read_manifest(output.path()).unwrap().roots,
            ["new-root.webui.js"]
        );

        publish(
            output.path(),
            &[
                file(NEXT_PAYLOAD, "next component"),
                file("next-root.webui.js", "next root"),
            ],
        )
        .unwrap();
        assert!(output.path().join(OLD_PAYLOAD).is_file());
        assert!(output.path().join(NEW_PAYLOAD).is_file());
        assert!(output.path().join(NEXT_PAYLOAD).is_file());
        fs::write(
            output.path().join("components/application.js"),
            "app payload",
        )
        .unwrap();
        prune(output.path()).unwrap();
        assert!(!output.path().join(OLD_PAYLOAD).exists());
        assert!(!output.path().join(NEW_PAYLOAD).exists());
        assert!(output.path().join(NEXT_PAYLOAD).is_file());
        assert!(output.path().join("components/application.js").is_file());
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
    fn finish_failure_restores_roots_but_preserves_payloads_observed_by_readers() {
        const PAYLOAD: &str = "components/new-card.bbbbbbbbbbbbbbbb.webui.js";
        let output = TempDir::new().unwrap();
        publish(output.path(), &[file("stable-root.webui.js", "old root")]).unwrap();
        let before = fs::read(manifest_path(output.path())).unwrap();

        let error = publish_with(
            output.path(),
            &[
                file("stable-root.webui.js", "new root"),
                file(PAYLOAD, "new payload"),
            ],
            || {
                assert_eq!(
                    fs::read_to_string(output.path().join("stable-root.webui.js")).unwrap(),
                    "new root"
                );
                assert!(output.path().join(PAYLOAD).is_file());
                Err(WebUIError::InvalidBuildOptions(
                    "metafile commit failed".to_string(),
                ))
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("metafile commit failed"));
        assert_eq!(
            fs::read_to_string(output.path().join("stable-root.webui.js")).unwrap(),
            "old root"
        );
        assert_eq!(fs::read(manifest_path(output.path())).unwrap(), before);
        assert!(output.path().join(PAYLOAD).is_file());
        prune(output.path()).unwrap();
        assert!(!output.path().join(PAYLOAD).exists());
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

    #[test]
    fn unchanged_graph_preserves_all_file_identities_and_runs_finish() {
        const PAYLOAD: &str = "components/test-card.0000000100000000.webui.js";
        let output = TempDir::new().unwrap();
        let files = [file("stable-root.webui.js", "root"), file(PAYLOAD, "a")];
        publish(output.path(), &files).unwrap();
        let paths = [
            output.path().join("stable-root.webui.js"),
            output.path().join(PAYLOAD),
            manifest_path(output.path()),
        ];
        let before = paths
            .each_ref()
            .map(|path| fs::metadata(path).unwrap().modified().unwrap());
        #[cfg(unix)]
        let inodes = paths.each_ref().map(|path| {
            use std::os::unix::fs::MetadataExt;
            fs::metadata(path).unwrap().ino()
        });
        let mut finished = false;
        publish_with(output.path(), &files, || {
            finished = true;
            Ok(())
        })
        .unwrap();
        assert!(finished);
        let after = paths
            .each_ref()
            .map(|path| fs::metadata(path).unwrap().modified().unwrap());
        assert_eq!(before, after);
        #[cfg(unix)]
        assert_eq!(
            inodes,
            paths.each_ref().map(|path| {
                use std::os::unix::fs::MetadataExt;
                fs::metadata(path).unwrap().ino()
            })
        );
    }

    #[test]
    fn invalid_paths_fail_before_writing_any_outputs() {
        let output = TempDir::new().unwrap();
        for name in [
            "../escape.webui.js",
            MANIFEST_FILE,
            "components/invalid.webui.js",
        ] {
            assert!(publish(output.path(), &[file(name, "invalid")]).is_err());
        }
        assert_eq!(fs::read_dir(output.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn publication_and_pruning_reject_symlinked_payload_directories() {
        let output = TempDir::new().unwrap();
        let external = TempDir::new().unwrap();
        std::os::unix::fs::symlink(external.path(), output.path().join("components")).unwrap();
        assert!(publish(output.path(), &[]).is_err());
        assert!(prune(output.path()).is_err());
        assert_eq!(fs::read_dir(external.path()).unwrap().count(), 0);
    }
}
