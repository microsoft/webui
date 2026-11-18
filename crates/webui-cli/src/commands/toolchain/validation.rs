// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::contract::{self, Target, MAX_METADATA, TYPESCRIPT_VERSION};
use super::{invalid, Toolchain, ToolchainError};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Metadata {
    schema_version: u32,
    version: String,
    target: String,
    typescript_version: String,
    #[serde(deserialize_with = "unique_files")]
    files: BTreeMap<String, String>,
}

pub(super) fn validate(root: &Path) -> Result<Toolchain, ToolchainError> {
    let target_name = env!("WEBUI_NATIVE_TARGET");
    let target = contract::target(target_name)
        .ok_or_else(|| ToolchainError::Unsupported(target_name.to_owned()))?;
    let kind = fs::symlink_metadata(root).map_err(|source| read_error(root, source))?;
    if !kind.is_dir() {
        return Err(invalid(
            root,
            "toolchain root must be a directory, not a symlink",
        ));
    }
    let root = fs::canonicalize(root).map_err(|source| read_error(root, source))?;
    let metadata_path = checked(&root, "toolchain.json")?;
    let bytes = contract::read_bounded(&metadata_path, MAX_METADATA)
        .map_err(|source| read_error(&metadata_path, source))?;
    let metadata: Metadata = serde_json::from_slice(&bytes)
        .map_err(|error| invalid(&metadata_path, error.to_string()))?;
    if metadata.schema_version != 1
        || metadata.version != env!("CARGO_PKG_VERSION")
        || metadata.target != target_name
        || metadata.typescript_version != TYPESCRIPT_VERSION
    {
        return Err(invalid(
            &metadata_path,
            format!(
                "expected schema 1, WebUI {}, target {target_name}, TypeScript {TYPESCRIPT_VERSION}",
                env!("CARGO_PKG_VERSION")
            ),
        ));
    }
    validate_inventory(&root, &metadata.files, &target)?;
    let compiler = root.join("compiler").join(target.compiler);
    for path in [&compiler, &root.join("bin").join(target.cli)] {
        if !contract::native_executable(path, &target).map_err(|source| read_error(path, source))? {
            return Err(invalid(
                path,
                "expected a target-matched native executable, not a script launcher",
            ));
        }
    }
    super::probe::check(&compiler)?;
    Ok(Toolchain { compiler })
}

fn validate_inventory(
    root: &Path,
    files: &BTreeMap<String, String>,
    target: &Target,
) -> Result<(), ToolchainError> {
    let expected: BTreeSet<_> = contract::inventory(target).collect();
    if files.len() != expected.len() || files.keys().any(|name| !expected.contains(name)) {
        return Err(invalid(root, "inventory must contain exactly the CLI, native compiler, all pinned standard libraries and notices"));
    }
    validate_tree(root, &expected)?;
    for (relative, expected_hash) in files {
        if expected_hash.len() != 64
            || !expected_hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid(root, format!("invalid SHA-256 for {relative}")));
        }
        let path = checked(root, relative)?;
        let digest = contract::sha256(&path, contract::file_limit(relative))
            .map_err(|source| read_error(&path, source))?;
        if &digest != expected_hash {
            return Err(invalid(&path, "toolchain integrity check failed"));
        }
    }
    Ok(())
}

fn validate_tree(root: &Path, files: &BTreeSet<String>) -> Result<(), ToolchainError> {
    let mut directories = vec![PathBuf::new()];
    let mut count = 0_usize;
    while let Some(relative) = directories.pop() {
        let directory = root.join(&relative);
        for entry in fs::read_dir(&directory).map_err(|source| read_error(&directory, source))? {
            let entry = entry.map_err(|source| read_error(&directory, source))?;
            count += 1;
            if count > files.len() + 4 {
                return Err(invalid(root, "unexpected toolchain inventory entries"));
            }
            let filename = entry.file_name();
            if filename
                .to_str()
                .is_none_or(|name| name.contains(['\\', ':']))
            {
                return Err(invalid(&entry.path(), "unsafe toolchain filename"));
            }
            let name = relative.join(filename);
            let name_text = name.to_string_lossy().replace('\\', "/");
            let kind = entry
                .file_type()
                .map_err(|source| read_error(&entry.path(), source))?;
            if kind.is_dir() && matches!(name_text.as_str(), "bin" | "compiler" | "notices") {
                directories.push(name);
            } else if !kind.is_file()
                || !(files.contains(&name_text) || name_text == "toolchain.json")
            {
                return Err(invalid(
                    &entry.path(),
                    "unknown entry or symlink in standalone toolchain",
                ));
            }
        }
    }
    Ok(())
}

fn checked(root: &Path, relative: &str) -> Result<PathBuf, ToolchainError> {
    contract::regular_file(root, relative).map_err(|source| {
        if source.kind() == std::io::ErrorKind::InvalidData {
            invalid(&root.join(relative), source.to_string())
        } else {
            read_error(&root.join(relative), source)
        }
    })
}

fn unique_files<'de, D>(deserializer: D) -> Result<BTreeMap<String, String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = BTreeMap<String, String>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a unique file-to-SHA-256 map")
        }
        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::MapAccess<'de>,
        {
            let mut files = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, String>()? {
                if files.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate inventory path"));
                }
            }
            Ok(files)
        }
    }
    deserializer.deserialize_map(Visitor)
}

#[cold]
#[inline(never)]
fn read_error(path: &Path, source: std::io::Error) -> ToolchainError {
    ToolchainError::Read {
        path: path.to_owned(),
        source,
    }
}
