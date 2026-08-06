// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Azure release orchestration commands.

mod artifacts;
mod git;
mod metadata;

use crate::version;
use git::RemoteTag;
use std::env;
use std::process::ExitCode;

/// Resolve the workspace release identity and emit Azure output variables.
pub fn run_resolve() -> ExitCode {
    finish(resolve())
}

/// Pack release artifacts and export the four Azure artifact directories.
pub fn run_package() -> ExitCode {
    finish(artifacts::package())
}

/// Write release metadata for the downstream official pipeline.
pub fn run_write_metadata() -> ExitCode {
    finish(metadata::write_from_env())
}

/// Read and validate release metadata, then emit Azure variables.
pub fn run_read_metadata() -> ExitCode {
    finish(metadata::read_from_env())
}

/// Stage downloaded pipeline artifacts into the ESRP input directories.
pub fn run_stage_artifacts() -> ExitCode {
    finish(artifacts::stage_from_env())
}

/// Create the annotated release tag, tolerating an identical concurrent tag.
pub fn run_ensure_tag() -> ExitCode {
    finish(git::ensure_tag_from_env())
}

fn finish(result: Result<(), String>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("  {} {message}", console::style("✘").red().bold());
            ExitCode::FAILURE
        }
    }
}

fn resolve() -> Result<(), String> {
    let source_branch = required_env("BUILD_SOURCEBRANCH")?;
    if !source_branch.starts_with("refs/heads/") {
        return Err("BUILD_SOURCEBRANCH must identify a branch as refs/heads/*.".to_string());
    }

    let release_commit = required_env("BUILD_SOURCEVERSION")?;
    validate_commit(&release_commit)?;
    let release_version = version::read_version()?;
    validate_stable_version(&release_version)?;
    let release_tag = format!("v{release_version}");
    let allow_existing = parse_bool(&required_env("ALLOW_EXISTING_RELEASE")?)?;

    let should_build = match git::remote_tag_commit(&release_tag)? {
        RemoteTag::Missing => true,
        RemoteTag::Commit(commit) if allow_existing => {
            println!(
                "Remote tag {release_tag} already exists at {commit}; rebuilding artifacts for validation."
            );
            true
        }
        RemoteTag::Commit(commit) => {
            println!(
                "Remote tag {release_tag} already exists at {commit}; skipping release build."
            );
            false
        }
    };

    println!("Resolved release: {release_tag} ({release_commit}); shouldBuild={should_build}");
    if let Ok(build_id) = env::var("BUILD_BUILDID") {
        if !build_id.is_empty() {
            println!("##vso[build.updatebuildnumber]{release_tag}-build-{build_id}");
        }
    }
    azure_output("releaseVersion", &release_version);
    azure_output("releaseTag", &release_tag);
    azure_output("releaseCommit", &release_commit);
    azure_output("shouldBuild", bool_text(should_build));
    Ok(())
}

pub(super) fn validate_release_tag(tag: &str) -> Result<(), String> {
    let Some(version) = tag.strip_prefix('v') else {
        return Err("release tag must exactly match vMAJOR.MINOR.PATCH".to_string());
    };
    validate_stable_version(version)
        .map_err(|_| "release tag must exactly match vMAJOR.MINOR.PATCH".to_string())
}

pub(super) fn validate_stable_version(value: &str) -> Result<(), String> {
    let mut parts = value.split('.');
    for _ in 0..3 {
        let Some(part) = parts.next() else {
            return Err("version must use stable MAJOR.MINOR.PATCH syntax".to_string());
        };
        if part.is_empty()
            || !part.bytes().all(|byte| byte.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
        {
            return Err("version must use stable MAJOR.MINOR.PATCH syntax".to_string());
        }
    }
    if parts.next().is_some() {
        return Err("version must use stable MAJOR.MINOR.PATCH syntax".to_string());
    }
    Ok(())
}

pub(super) fn validate_commit(value: &str) -> Result<(), String> {
    if value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err("commit must be a 40-character lowercase hexadecimal ID".to_string())
    }
}

pub(super) fn parse_bool(value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err("validation mode must be true or false".to_string()),
    }
}

pub(super) fn required_env(name: &str) -> Result<String, String> {
    match env::var(name) {
        Ok(value) if !value.is_empty() => Ok(value),
        Ok(_) | Err(env::VarError::NotPresent) => Err(format!("{name} is required")),
        Err(error) => Err(format!("{name} is not valid Unicode: {error}")),
    }
}

pub(super) fn bool_text(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

fn azure_output(name: &str, value: &str) {
    println!("##vso[task.setvariable variable={name};isOutput=true]{value}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_version_rejects_prerelease_and_leading_zeroes() {
        assert!(validate_stable_version("1.2.3").is_ok());
        assert!(validate_stable_version("1.2.3-beta").is_err());
        assert!(validate_stable_version("01.2.3").is_err());
        assert!(validate_stable_version("1.2").is_err());
    }
}
