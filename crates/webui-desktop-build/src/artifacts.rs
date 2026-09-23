// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::{
    error::{io, schema},
    GenerateConfig, GenerateError,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};
mod paths;
mod transaction;
use paths::Scope;
use transaction::Change;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Inventory {
    rust: Vec<PathBuf>,
    typescript: Vec<PathBuf>,
}

pub(crate) fn validate_paths(config: &GenerateConfig) -> Result<(), GenerateError> {
    Scope::new(&config.rust_out)?;
    Scope::new(&config.ts_out)?;
    let parent = config
        .lock_file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let metadata = Scope::new(parent)?;
    for path in [
        config.lock_file.clone(),
        config.lock_file.with_file_name("ipc-schema.json"),
        config.lock_file.with_file_name("ipc-generated-files.json"),
    ] {
        metadata.destination(&path)?;
    }
    Ok(())
}

pub(crate) fn persist(
    config: &GenerateConfig,
    mut artifacts: BTreeMap<PathBuf, Vec<u8>>,
) -> Result<(), GenerateError> {
    let index = config.lock_file.with_file_name("ipc-generated-files.json");
    let manifest = config.lock_file.with_file_name("ipc-schema.json");
    let metadata_root = config
        .lock_file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let metadata = Scope::new(metadata_root)?;
    let rust = Scope::new(&config.rust_out)?;
    let typescript = Scope::new(&config.ts_out)?;
    metadata.destination(&index)?;
    metadata.destination(&config.lock_file)?;
    metadata.destination(&manifest)?;
    if artifacts.contains_key(&index) {
        return Err(schema(
            "ipc-output-collision",
            index.display().to_string(),
            "artifact collides with the publication inventory",
            "choose a distinct lock file and output paths",
        ));
    }
    let inventory = Inventory {
        rust: relative_files(&artifacts, &config.rust_out, "rs"),
        typescript: relative_files(&artifacts, &config.ts_out, "ts"),
    };
    let stale = match fs::read(&index) {
        Ok(bytes) => {
            let previous: Inventory = serde_json::from_slice(&bytes).map_err(|e| {
                schema(
                    "ipc-inventory",
                    index.display().to_string(),
                    e.to_string(),
                    "restore the generated artifact inventory",
                )
            })?;
            let mut stale = stale_files(&config.rust_out, &previous.rust, &inventory.rust)?;
            stale.extend(stale_files(
                &config.ts_out,
                &previous.typescript,
                &inventory.typescript,
            )?);
            stale
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(io(&index, e)),
    };
    let mut inventory_bytes = serde_json::to_vec_pretty(&inventory).map_err(|e| {
        schema(
            "ipc-inventory",
            "generated files",
            e.to_string(),
            "report this generator error",
        )
    })?;
    inventory_bytes.push(b'\n');
    artifacts.insert(index.clone(), inventory_bytes);
    let mut drift = stale.clone();
    let mut changes = Vec::new();
    let mut destinations = BTreeSet::new();
    for (path, content) in artifacts {
        let (scope, order) = if path == index {
            (&metadata, 3)
        } else if path == config.lock_file {
            (&metadata, 2)
        } else if path == manifest {
            (&metadata, 1)
        } else if path.starts_with(&config.rust_out) && path.extension().is_some_and(|e| e == "rs")
        {
            (&rust, 0)
        } else {
            (&typescript, 0)
        };
        let destination = scope.destination(&path)?;
        if !destinations.insert(destination.path.clone()) {
            return Err(schema(
                "ipc-output-collision",
                path.display().to_string(),
                "multiple artifacts resolve to the same destination",
                "use distinct output and metadata paths",
            ));
        }
        let existing = match fs::read(&destination.path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(io(&path, e)),
        };
        if existing.as_deref() != Some(content.as_slice()) {
            drift.push(path.clone());
            changes.push(Change {
                destination,
                before: existing,
                after: Some(content),
                order,
            });
        }
    }
    for path in stale {
        let scope = if path.starts_with(&config.rust_out) {
            &rust
        } else {
            &typescript
        };
        let destination = scope.destination(&path)?;
        if !destinations.insert(destination.path.clone()) {
            return Err(schema(
                "ipc-output-collision",
                path.display().to_string(),
                "obsolete artifact overlaps a current artifact",
                "use disjoint artifact paths",
            ));
        }
        let before = fs::read(&destination.path).map_err(|e| io(&path, e))?;
        changes.push(Change {
            destination,
            before: Some(before),
            after: None,
            order: 0,
        });
    }
    if config.check {
        return if drift.is_empty() {
            Ok(())
        } else {
            Err(GenerateError::Drift { paths: drift })
        };
    }
    transaction::publish(changes)
}

fn relative_files(
    artifacts: &BTreeMap<PathBuf, Vec<u8>>,
    root: &Path,
    extension: &str,
) -> Vec<PathBuf> {
    artifacts
        .keys()
        .filter(|p| p.extension().is_some_and(|e| e == extension))
        .filter_map(|p| p.strip_prefix(root).ok().map(Path::to_owned))
        .map(|path| PathBuf::from(path.to_string_lossy().replace('\\', "/")))
        .collect()
}

fn stale_files(
    root: &Path,
    previous: &[PathBuf],
    current: &[PathBuf],
) -> Result<Vec<PathBuf>, GenerateError> {
    let mut stale = Vec::new();
    for relative in previous {
        if relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(schema(
                "ipc-inventory",
                relative.display().to_string(),
                "inventory path escapes its output directory",
                "restore the generated inventory; paths must be relative without parent components",
            ));
        }
        if current.contains(relative) {
            continue;
        }
        let path = root.join(relative);
        Scope::new(root)?.destination(&path)?;
        if !path.exists() {
            continue;
        }
        let content = fs::read_to_string(&path).map_err(|e| io(&path, e))?;
        if !content.lines().take(8).any(|line| {
            line.contains("@generated") || line.contains("Code generated by protoc-gen-ts_proto")
        }) {
            return Err(schema(
                "ipc-stale-artifact",
                path.display().to_string(),
                "stale artifact no longer has a generated header",
                "move manually maintained files out of the generated output directory",
            ));
        }
        stale.push(path);
    }
    Ok(stale)
}
