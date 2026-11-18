// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! CI/build-time checker sourcing; never used by the installed consumer.

#[path = "native_toolchain/source.rs"]
mod source;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::native_toolchain::{self, StageOptions};

const USAGE: &str = "\
Usage: cargo xtask toolchain-stage --target <cargo-triple> --cli <built-cli> \
(--checker-package <native-package.tgz> | --acquire-checker) [--checker-dir <unpacked-package>]

The checker is pinned to TypeScript 7.0.2 and SHA-512 from pnpm-lock.yaml.
--acquire-checker explicitly downloads the requested target's official microsoft/TypeScript
GitHub release asset using an isolated Go-stdlib HTTPS producer helper (Go required).
--checker-dir additionally verifies the unpacked files against that verified tarball.
No target executables, JavaScript launchers, Node or npm are run.
WebUI version comes from Cargo.toml. Inputs must already be built/transformed.
Validate the complete assembled distribution: cargo xtask toolchain-stage --validate-release";

struct Options {
    target: String,
    cli: PathBuf,
    package: Option<PathBuf>,
    checker_dir: Option<PathBuf>,
}

/// Stage a standalone native toolchain from explicit, integrity-checked inputs.
pub(crate) fn run(root: &Path, args: &[String]) -> ExitCode {
    if args == ["--help"] {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match execute(root, args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!(
                "{} {error}\n\n{USAGE}",
                console::style("toolchain staging failed:").red()
            );
            ExitCode::FAILURE
        }
    }
}

fn execute(root: &Path, args: &[String]) -> Result<(), String> {
    if args == ["--validate-release"] {
        let version = crate::version::workspace_package_version(root)?;
        return native_toolchain::validate_release(root, &version);
    }
    let options = parse(args)?;
    let version = crate::version::workspace_package_version(root)?;
    crate::release_version::ReleaseVersion::parse(&version)
        .ok_or("Cargo.toml does not contain a valid WebUI release version")?;
    let temporary = tempfile::tempdir().map_err(|e| e.to_string())?;
    let checker = source::prepare(&source::SourceOptions {
        root,
        target: &options.target,
        package: options.package.as_deref(),
        checker_dir: options.checker_dir.as_deref(),
        temporary: temporary.path(),
    })?;
    let staged = native_toolchain::stage(&StageOptions {
        root,
        version: &version,
        target: &options.target,
        checker_dir: &checker.directory,
        cli: &options.cli,
    })?;
    println!(
        "Checker: {} ({}, pnpm-lock.yaml)",
        checker.package, checker.integrity
    );
    println!("Archive: {}", staged.archive.display());
    println!("SHA-256: {}", staged.checksum.display());
    Ok(())
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut values = BTreeMap::new();
    let mut acquire = false;
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        if flag == "--acquire-checker" {
            if std::mem::replace(&mut acquire, true) {
                return Err("duplicate --acquire-checker".to_owned());
            }
        } else {
            if !matches!(
                flag.as_str(),
                "--target" | "--cli" | "--checker-package" | "--checker-dir"
            ) {
                return Err(format!("unknown argument {flag}"));
            }
            let value = args
                .next()
                .filter(|value| !value.starts_with("--"))
                .ok_or_else(|| format!("{flag} requires a value"))?;
            if value.is_empty() || values.insert(flag.as_str(), value.as_str()).is_some() {
                return Err(format!("empty or duplicate {flag}"));
            }
        }
    }
    let required = |flag| {
        values
            .get(flag)
            .copied()
            .ok_or_else(|| format!("missing {flag}"))
    };
    let target = required("--target")?.to_owned();
    let cli = PathBuf::from(required("--cli")?);
    let package = values.get("--checker-package").map(PathBuf::from);
    if acquire == package.is_some() {
        return Err("choose exactly one of --checker-package or --acquire-checker; an unpacked directory alone has no verifiable tarball provenance".to_owned());
    }
    Ok(Options {
        target,
        cli,
        package,
        checker_dir: values.get("--checker-dir").map(PathBuf::from),
    })
}

#[cfg(test)]
#[path = "native_toolchain/command_tests.rs"]
mod tests;
