// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};

use super::hash::{hash_file, HASH_BUFFER_SIZE};
use super::should_ignore_event;

pub(super) fn initial_hashes(
    roots: &[PathBuf],
    ignore: &[PathBuf],
    explicit: &[PathBuf],
) -> Result<HashMap<PathBuf, u64>> {
    let roots = roots
        .iter()
        .filter(|path| path.exists())
        .map(|path| {
            path.canonicalize()
                .with_context(|| format!("Cannot resolve watcher input {}", path.display()))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut pending = Vec::with_capacity(roots.len() + explicit.len());
    for root in roots.iter().chain(explicit) {
        if root.exists() {
            pending.push(
                root.canonicalize()
                    .with_context(|| format!("Cannot resolve watcher input {}", root.display()))?,
            );
        }
    }
    let mut hashes = HashMap::new();
    let mut buffer = [0_u8; HASH_BUFFER_SIZE];
    while let Some(path) = pending.pop() {
        if should_ignore_event(&path, ignore, explicit, &roots) || hashes.contains_key(&path) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("Cannot inspect watcher input {}", path.display()))?;
        if metadata.is_dir() {
            let entries = std::fs::read_dir(&path)
                .with_context(|| format!("Cannot enumerate watcher input {}", path.display()))?;
            for entry in entries {
                pending.push(entry.context("Cannot enumerate watcher input")?.path());
            }
        } else if metadata.is_file() {
            if let Some(hash) = hash_file(&path, &mut buffer) {
                hashes.insert(path, hash);
            }
        }
    }
    Ok(hashes)
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn seeds_source_and_explicit_inputs_but_not_outputs() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("index.html");
        let output = root.path().join("dist");
        std::fs::create_dir(&output).unwrap();
        let manifest = output.join("manifest.json");
        std::fs::write(&input, "source").unwrap();
        std::fs::write(&manifest, "{}").unwrap();
        std::fs::write(output.join("bundle.js"), "output").unwrap();
        let input = input.canonicalize().unwrap();
        let manifest = manifest.canonicalize().unwrap();
        let mut hashes = initial_hashes(
            &[root.path().to_owned()],
            &[output.canonicalize().unwrap()],
            &[manifest.clone()],
        )
        .unwrap();
        assert_eq!(hashes.len(), 2);
        let mut buffer = [0_u8; HASH_BUFFER_SIZE];
        assert!(!super::super::should_forward_path(
            &mut hashes,
            &input,
            false,
            &mut buffer
        ));
        assert!(!super::super::should_forward_path(
            &mut hashes,
            &manifest,
            false,
            &mut buffer
        ));
        std::fs::write(&input, "changed").unwrap();
        assert!(super::super::should_forward_path(
            &mut hashes,
            &input,
            false,
            &mut buffer
        ));
        assert!(super::super::should_forward_path(
            &mut hashes,
            &input,
            true,
            &mut buffer
        ));
    }
}
