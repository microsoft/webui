// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Explicit-input staging only. No acquisition, build, Node, or publication.
//!
//! The CLI owns this archive format and its pinned stdlib inventory. Share its
//! host-independent contract helpers rather than maintaining a second policy.
#[path = "../../crates/webui-cli/src/commands/toolchain/contract.rs"]
pub(crate) mod contract;
#[path = "native_toolchain/producer_go.rs"]
pub(crate) mod producer_go;
#[path = "native_toolchain/release.rs"]
mod release;
#[cfg(test)]
#[path = "native_toolchain/test_fixture.rs"]
mod test_fixture;

pub(crate) use release::validate_release;

/// Stage six synthetic, non-executable archive-layout fixtures with real
/// inventories/checksums. Test-only; not source or native compilation proof.
#[cfg(test)]
pub(crate) fn test_release_fixture(root: &std::path::Path, version: &str) -> Result<(), String> {
    test_fixture::stage(root, version)
}

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Map, Value};

/// All artifact sources are supplied explicitly by the build/release caller.
pub(crate) struct StageOptions<'a> {
    /// Workspace root (WebUI LICENSE source and isolated publish output owner).
    pub(crate) root: &'a Path,
    /// WebUI release version, distinct from the pinned upstream checker version.
    pub(crate) version: &'a str,
    /// Cargo target triple of both supplied native executables.
    pub(crate) target: &'a str,
    /// Unpacked upstream native package with package.json, lib/, LICENSE, NOTICE.txt.
    pub(crate) checker_dir: &'a Path,
    /// Already-built, already-transformed/signed CLI binary for this release.
    pub(crate) cli: &'a Path,
}

/// Locally staged versioned archive and its adjacent SHA-256 sidecar.
pub(crate) struct StagedToolchain {
    pub(crate) archive: PathBuf,
    pub(crate) checksum: PathBuf,
}

/// Assemble the standalone archive from final local native artifacts.
/// A caller must establish upstream provenance and the CLI's release identity.
/// Cross-target staging verifies format/architecture, but never executes inputs.
pub(crate) fn stage(options: &StageOptions<'_>) -> Result<StagedToolchain, String> {
    validate_version(options.version)?;
    let target = contract::target(options.target)
        .ok_or_else(|| format!("unsupported native toolchain target {}", options.target))?;
    let source = canonical_directory(options.checker_dir)?;
    validate_package(&source, &target)?;
    let cli = regular_input(options.cli)?;
    for path in [
        &cli,
        &contract::regular_file(&source, &format!("lib/{}", target.compiler))
            .map_err(|e| e.to_string())?,
    ] {
        if !contract::native_executable(path, &target).map_err(|e| e.to_string())? {
            return Err(format!(
                "expected target-matched native binary, not a launcher: {}",
                path.display()
            ));
        }
    }
    let output = output_directory(options.root)?;
    let temporary = tempfile::tempdir_in(&output).map_err(|e| e.to_string())?;
    let content = temporary.path().join("content");
    fs::create_dir(&content).map_err(|e| e.to_string())?;
    assemble(options, &source, &cli, &content, &target)?;
    let filename = format!(
        "webui-toolchain-{}-{}.tar.gz",
        options.version, options.target
    );
    let archive = temporary.path().join(&filename);
    let result = Command::new("tar")
        .env("COPYFILE_DISABLE", "1")
        .args(["-czf"])
        .arg(&archive)
        .arg("-C")
        .arg(&content)
        .args(["bin", "compiler", "notices", "toolchain.json"])
        .output()
        .map_err(|e| format!("standalone toolchain staging requires tar on PATH: {e}"))?;
    if !result.status.success() {
        return Err(format!(
            "toolchain archive failed: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    let digest = contract::sha256(&archive, 1024 * 1024 * 1024).map_err(|e| e.to_string())?;
    let checksum = temporary.path().join("checksum");
    fs::write(&checksum, format!("{digest}  {filename}\n")).map_err(|e| e.to_string())?;
    let final_archive = output.join(&filename);
    let final_checksum = output.join(format!("{filename}.sha256"));
    // Validate and assemble fully before replacing either published local file.
    // Publication/aggregation and release authenticity remain caller-owned.
    fs::rename(archive, &final_archive).map_err(|e| e.to_string())?;
    fs::rename(checksum, &final_checksum).map_err(|e| e.to_string())?;
    Ok(StagedToolchain {
        archive: final_archive,
        checksum: final_checksum,
    })
}

fn assemble(
    options: &StageOptions<'_>,
    source: &Path,
    cli: &Path,
    output: &Path,
    target: &contract::Target,
) -> Result<(), String> {
    let workspace = fs::canonicalize(options.root).map_err(|e| e.to_string())?;
    for directory in ["bin", "compiler", "notices"] {
        fs::create_dir(output.join(directory)).map_err(|e| e.to_string())?;
    }
    let mut files = Map::new();
    for relative in contract::inventory(target) {
        let input = match relative.as_str() {
            name if name.starts_with("bin/") => Ok(cli.to_owned()),
            "notices/WebUI-LICENSE" => contract::regular_file(&workspace, "LICENSE"),
            "notices/TypeScript-LICENSE" => contract::regular_file(source, "LICENSE"),
            "notices/TypeScript-NOTICE.txt" => contract::regular_file(source, "NOTICE.txt"),
            name => contract::regular_file(source, &format!("lib/{}", &name["compiler/".len()..])),
        };
        let input = input.map_err(|e| format!("cannot stage {relative}: {e}"))?;
        if fs::metadata(&input).map_err(|e| e.to_string())?.len() > contract::file_limit(&relative)
        {
            return Err(format!(
                "source artifact exceeds its size limit: {relative}"
            ));
        }
        let destination = output.join(&relative);
        fs::copy(&input, &destination)
            .map_err(|e| format!("cannot copy {}: {e}", input.display()))?;
        // All transforms/signing must precede inventory generation.
        let digest = contract::sha256(&destination, contract::file_limit(&relative))
            .map_err(|e| format!("cannot hash {relative}: {e}"))?;
        files.insert(relative, Value::from(digest));
    }
    let metadata = Value::Object(Map::from_iter([
        ("schemaVersion".to_owned(), Value::from(1)),
        ("version".to_owned(), Value::from(options.version)),
        ("target".to_owned(), Value::from(options.target)),
        (
            "typescriptVersion".to_owned(),
            Value::from(contract::TYPESCRIPT_VERSION),
        ),
        ("files".to_owned(), Value::Object(files)),
    ]));
    let bytes = serde_json::to_vec_pretty(&metadata).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > contract::MAX_METADATA {
        return Err("toolchain metadata exceeds 64 KiB".to_owned());
    }
    fs::write(output.join("toolchain.json"), bytes).map_err(|e| e.to_string())
}

fn validate_package(source: &Path, target: &contract::Target) -> Result<(), String> {
    let path = contract::regular_file(source, "package.json").map_err(|e| e.to_string())?;
    let bytes = contract::read_bounded(&path, contract::MAX_METADATA).map_err(|e| e.to_string())?;
    let metadata: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let expected_name = format!("@typescript/typescript-{}-{}", target.os, target.cpu);
    if metadata["name"].as_str() != Some(&expected_name)
        || metadata["version"].as_str() != Some(contract::TYPESCRIPT_VERSION)
        || metadata["os"]
            .as_array()
            .is_none_or(|v| v.len() != 1 || v[0] != target.os)
        || metadata["cpu"]
            .as_array()
            .is_none_or(|v| v.len() != 1 || v[0] != target.cpu)
    {
        return Err(format!(
            "checker source must be {expected_name}@{}",
            contract::TYPESCRIPT_VERSION
        ));
    }
    // Fail on missing or unexpected stdlibs rather than silently copying a
    // partially installed/mixed upstream distribution.
    let mut actual = std::collections::BTreeSet::new();
    let directory = source.join("lib");
    if !fs::symlink_metadata(&directory)
        .map_err(|e| e.to_string())?
        .is_dir()
    {
        return Err("checker lib directory must not be a symlink".to_owned());
    }
    let expected: std::collections::BTreeSet<_> = contract::libraries().collect();
    for (count, entry) in fs::read_dir(&directory)
        .map_err(|e| e.to_string())?
        .enumerate()
    {
        if count > expected.len() + 16 {
            return Err("unexpected checker source directory entries".to_owned());
        }
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let name = name.to_str().ok_or("non-UTF8 checker source filename")?;
        if name.starts_with("lib") && name.ends_with(".d.ts") {
            if !expected.contains(name) || !entry.file_type().map_err(|e| e.to_string())?.is_file()
            {
                return Err(format!("unexpected or symlinked checker library {name}"));
            }
            actual.insert(name.to_owned());
        }
    }
    if actual.len() != expected.len() {
        return Err("checker source is missing pinned TypeScript standard libraries".to_owned());
    }
    Ok(())
}

fn output_directory(root: &Path) -> Result<PathBuf, String> {
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let mut output = root;
    for relative in ["publish", "toolchains"] {
        output.push(relative);
        match fs::symlink_metadata(&output) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                return Err(format!(
                    "staging output must not be a symlink: {}",
                    output.display()
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&output).map_err(|e| e.to_string())?;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(output)
}

fn canonical_directory(path: &Path) -> Result<PathBuf, String> {
    if !fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .is_dir()
    {
        return Err(format!(
            "checker source must be a directory, not a symlink: {}",
            path.display()
        ));
    }
    fs::canonicalize(path).map_err(|e| e.to_string())
}

fn regular_input(path: &Path) -> Result<PathBuf, String> {
    if !fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .is_file()
    {
        return Err(format!(
            "CLI source must be a regular file, not a symlink: {}",
            path.display()
        ));
    }
    fs::canonicalize(path).map_err(|e| e.to_string())
}

fn validate_version(version: &str) -> Result<(), String> {
    if version.is_empty()
        || version.len() > 80
        || !version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
        || !version.as_bytes()[0].is_ascii_digit()
    {
        return Err("invalid standalone WebUI release version".to_owned());
    }

    Ok(())
}

#[cfg(test)]
#[path = "native_toolchain/tests.rs"]
mod tests;
