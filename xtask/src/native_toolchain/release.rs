// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Portable release validation. Target CLI/checker executables are only read.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Stdio;

use serde_json::{Map, Value};

use super::{contract, producer_go};

pub(super) const TARGETS: [&str; 6] = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
];

/// Require the complete six-target archive/checksum distribution, validating
/// compressed hashes, safe tar membership, metadata and every payload hash.
/// Go is a build-time host helper only; no target executable is ever launched.
pub(crate) fn validate_release(root: &Path, version: &str) -> Result<(), String> {
    super::validate_version(version)?;
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    for relative in ["publish", "publish/toolchains"] {
        let path = root.join(relative);
        if !fs::symlink_metadata(&path)
            .map_err(|e| e.to_string())?
            .is_dir()
        {
            return Err(format!(
                "toolchain distribution must not be a symlink: {}",
                path.display()
            ));
        }
    }
    let directory = root.join("publish/toolchains");
    let expected: BTreeSet<_> = TARGETS
        .iter()
        .flat_map(|target| {
            let archive = filename(version, target);
            [archive.clone(), format!("{archive}.sha256")]
        })
        .collect();
    let mut actual = BTreeSet::new();
    for entry in fs::read_dir(&directory).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_file()
            || !expected.contains(&entry.file_name().to_string_lossy().to_string())
            || actual.len() >= expected.len()
        {
            return Err(
                "expected exactly six native toolchain archives and six regular SHA-256 sidecars"
                    .to_owned(),
            );
        }
        actual.insert(entry.file_name().to_string_lossy().into_owned());
    }
    if actual != expected {
        return Err("incomplete native toolchain release: all six targets and SHA-256 sidecars are mandatory".to_owned());
    }
    validate_archives(&directory, version, &TARGETS)
}

fn filename(version: &str, target: &str) -> String {
    format!("webui-toolchain-{version}-{target}.tar.gz")
}

fn validate_archives(directory: &Path, version: &str, targets: &[&str]) -> Result<(), String> {
    let temporary = tempfile::tempdir().map_err(|e| e.to_string())?;
    let mut specifications = Vec::with_capacity(targets.len());
    for target in targets {
        let name = filename(version, target);
        let archive = contract::regular_file(directory, &name).map_err(|e| e.to_string())?;
        let checksum = contract::regular_file(directory, &format!("{name}.sha256"))
            .map_err(|e| e.to_string())?;
        let actual = contract::sha256(&archive, 1024 * 1024 * 1024).map_err(|e| e.to_string())?;
        let expected = contract::read_bounded(&checksum, 1024).map_err(|e| e.to_string())?;
        if expected != format!("{actual}  {name}\n").as_bytes() {
            return Err(format!("native toolchain archive SHA-256 mismatch: {name}"));
        }
        specifications.push(specification(
            &archive,
            temporary.path(),
            version,
            target,
            &actual,
        )?);
    }
    let spec = temporary.path().join("release.json");
    let helper = temporary.path().join("validate.go");
    fs::write(
        &spec,
        serde_json::to_vec(&specifications).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::write(&helper, include_str!("validate.go")).map_err(|e| e.to_string())?;
    let status = producer_go::command(temporary.path())?
        .args(["run", "-p=2", "-trimpath"])
        .arg(helper)
        .arg(spec)
        .stdin(Stdio::null())
        .status()
        .map_err(|e| format!("portable toolchain validation requires Go: {e}"))?;
    if !status.success() {
        return Err(format!(
            "native toolchain archive/inventory validation failed ({status})"
        ));
    }
    for triple in targets {
        let target = contract::target(triple).ok_or("unsupported toolchain target")?;
        for binary in [
            format!("bin/{}", target.cli),
            format!("compiler/{}", target.compiler),
        ] {
            let path = temporary.path().join(triple).join(binary);
            if !contract::native_executable(&path, &target).map_err(|e| e.to_string())? {
                return Err(format!(
                    "toolchain {triple} contains a nonnative or wrong-architecture executable"
                ));
            }
        }
    }
    Ok(())
}

fn specification(
    archive: &Path,
    temporary: &Path,
    version: &str,
    triple: &str,
    digest: &str,
) -> Result<Value, String> {
    let target = contract::target(triple).ok_or("unsupported toolchain target")?;
    let files = Map::from_iter(contract::inventory(&target).map(|name| {
        let limit = contract::file_limit(&name);
        (name, Value::from(limit))
    }));
    Ok(Value::Object(Map::from_iter([
        (
            "archive".to_owned(),
            Value::from(archive.to_string_lossy().as_ref()),
        ),
        ("sha256".to_owned(), Value::from(digest)),
        (
            "output".to_owned(),
            Value::from(temporary.join(triple).to_string_lossy().as_ref()),
        ),
        ("version".to_owned(), Value::from(version)),
        ("target".to_owned(), Value::from(triple)),
        (
            "typescriptVersion".to_owned(),
            Value::from(contract::TYPESCRIPT_VERSION),
        ),
        ("files".to_owned(), Value::Object(files)),
        (
            "binaries".to_owned(),
            Value::Array(vec![
                Value::from(format!("bin/{}", target.cli)),
                Value::from(format!("compiler/{}", target.compiler)),
            ]),
        ),
    ])))
}

#[cfg(test)]
#[path = "release_tests.rs"]
mod tests;
