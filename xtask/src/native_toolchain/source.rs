// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Frozen-lockfile SHA-512 verification precedes all archive interpretation.

#[path = "source_archive.rs"]
mod archive;
#[path = "https.rs"]
mod https;

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use sha2::{Digest, Sha512};

use crate::native_toolchain::contract::{self, Target, TYPESCRIPT_VERSION};

const MAX_PACKAGE: u64 = 128 * 1024 * 1024;

/// Explicit build-time inputs; no package-store discovery is performed.
pub(super) struct SourceOptions<'a> {
    pub(super) root: &'a Path,
    pub(super) target: &'a str,
    /// None means explicitly requested acquisition, never an implicit fallback.
    pub(super) package: Option<&'a Path>,
    pub(super) checker_dir: Option<&'a Path>,
    pub(super) temporary: &'a Path,
}

/// A private extracted copy of the exact digest-verified upstream artifact.
pub(super) struct CheckerSource {
    pub(super) directory: PathBuf,
    pub(super) package: String,
    pub(super) integrity: String,
}

struct LockedChecker {
    package: String,
    url: String,
    integrity: String,
    digest: Vec<u8>,
}

/// Verify the pinned archive before interpretation, optionally comparing an
/// explicitly supplied directory. Always stage from the verified private copy.
pub(super) fn prepare(options: &SourceOptions<'_>) -> Result<CheckerSource, String> {
    let target = contract::target(options.target)
        .ok_or_else(|| format!("unsupported checker target {}", options.target))?;
    let locked = locked_checker(options.root, &target)?;
    let package = options.temporary.join("checker.tgz");
    match options.package {
        Some(input) => copy_package(input, &package)?,
        None => https::acquire(&locked.url, &package)?,
    }
    verify(&package, &locked.digest)?;
    let directory = options.temporary.join("checker");
    let files = source_files(&target);
    archive::extract(&package, &directory, &files)?;
    if let Some(explicit) = options.checker_dir {
        compare_directory(explicit, &directory, &files)?;
    }
    Ok(CheckerSource {
        directory,
        package: locked.package,
        integrity: locked.integrity,
    })
}

fn locked_checker(root: &Path, target: &Target) -> Result<LockedChecker, String> {
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let lockfile = contract::regular_file(&root, "pnpm-lock.yaml").map_err(|e| e.to_string())?;
    let bytes = contract::read_bounded(&lockfile, 2 * 1024 * 1024).map_err(|e| e.to_string())?;
    let package_name = format!("typescript-{}-{}", target.os, target.cpu);
    let package = format!("@typescript/{package_name}@{TYPESCRIPT_VERSION}");
    // pnpm 12 writes a package-manager document followed by the workspace
    // dependency document. Honor both without conflating duplicate identities.
    let mut integrity = None;
    for (count, document) in serde_yaml::Deserializer::from_slice(&bytes).enumerate() {
        if count >= 8 {
            return Err("frozen lockfile exceeds eight YAML documents".to_owned());
        }
        let lock = serde_yaml::Value::deserialize(document).map_err(|e| e.to_string())?;
        let entry = &lock["packages"][package.as_str()];
        if !entry.is_null() {
            let value = entry["resolution"]["integrity"]
                .as_str()
                .ok_or_else(|| format!("missing frozen SHA-512 resolution for {package}"))?;
            if integrity.replace(value.to_owned()).is_some() {
                return Err(format!(
                    "ambiguous duplicate frozen resolution for {package}"
                ));
            }
        }
    }
    let integrity =
        integrity.ok_or_else(|| format!("missing frozen SHA-512 resolution for {package}"))?;
    let encoded = integrity
        .strip_prefix("sha512-")
        .ok_or_else(|| format!("expected SHA-512 integrity for {package}"))?;
    let digest = STANDARD
        .decode(encoded)
        .map_err(|e| format!("invalid frozen package integrity: {e}"))?;
    if digest.len() != 64 {
        return Err(format!("expected a 64-byte SHA-512 digest for {package}"));
    }
    Ok(LockedChecker {
        package,
        url: format!("https://github.com/microsoft/TypeScript/releases/download/v{TYPESCRIPT_VERSION}/{package_name}.tgz"),
        integrity,
        digest,
    })
}

fn copy_package(input: &Path, output: &Path) -> Result<(), String> {
    if !fs::symlink_metadata(input)
        .map_err(|e| e.to_string())?
        .is_file()
    {
        return Err("checker package must be a regular tarball, not a symlink".to_owned());
    }
    let mut input = File::open(input)
        .map_err(|e| e.to_string())?
        .take(MAX_PACKAGE + 1);
    let mut output = File::create_new(output).map_err(|e| e.to_string())?;
    let copied = std::io::copy(&mut input, &mut output).map_err(|e| e.to_string())?;
    output.flush().map_err(|e| e.to_string())?;
    if copied > MAX_PACKAGE {
        return Err("checker package exceeds 128 MiB".to_owned());
    }
    Ok(())
}

fn verify(package: &Path, expected: &[u8]) -> Result<(), String> {
    let mut file = File::open(package)
        .map_err(|e| e.to_string())?
        .take(MAX_PACKAGE + 1);
    let mut hash = Sha512::new();
    let mut buffer = vec![0_u8; 16 * 1024];
    let mut total = 0_u64;
    loop {
        let length = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if length == 0 {
            break;
        }
        total += length as u64;
        if total > MAX_PACKAGE {
            return Err("checker package exceeds 128 MiB".to_owned());
        }
        hash.update(&buffer[..length]);
    }
    if hash.finalize().as_slice() != expected {
        return Err("checker package SHA-512 mismatch; restore the exact 7.0.2 target artifact pinned by pnpm-lock.yaml".to_owned());
    }
    Ok(())
}

fn source_files(target: &Target) -> BTreeSet<String> {
    ["package.json", "LICENSE", "NOTICE.txt"]
        .map(str::to_owned)
        .into_iter()
        .chain(std::iter::once(format!("lib/{}", target.compiler)))
        .chain(contract::libraries().map(|name| format!("lib/{name}")))
        .collect()
}

fn compare_directory(
    directory: &Path,
    verified: &Path,
    files: &BTreeSet<String>,
) -> Result<(), String> {
    if !fs::symlink_metadata(directory)
        .map_err(|e| e.to_string())?
        .is_dir()
    {
        return Err("explicit checker directory must not be a symlink".to_owned());
    }
    let directory = fs::canonicalize(directory).map_err(|e| e.to_string())?;
    for relative in files {
        let local = contract::regular_file(&directory, relative).map_err(|e| e.to_string())?;
        let limit = match relative.as_str() {
            "package.json" => contract::MAX_METADATA,
            "LICENSE" | "NOTICE.txt" => 8 * 1024 * 1024,
            _ => contract::file_limit(relative),
        };
        let actual = contract::sha256(&local, limit).map_err(|e| e.to_string())?;
        let expected =
            contract::sha256(&verified.join(relative), limit).map_err(|e| e.to_string())?;
        if actual != expected {
            return Err(format!("explicit checker directory differs from the verified upstream artifact: {relative}"));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
