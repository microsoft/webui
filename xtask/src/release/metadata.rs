// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{
    bool_text, parse_bool, required_env, validate_commit, validate_release_tag,
    validate_stable_version,
};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const METADATA_FILES: &[(&str, &str)] = &[
    ("release-tag.txt", "release tag"),
    ("release-commit.txt", "release commit"),
    ("release-version.txt", "release version"),
    ("validation-mode.txt", "validation mode"),
];

#[derive(Debug, Eq, PartialEq)]
struct ReleaseMetadata {
    tag: String,
    commit: String,
    version: String,
    validation_mode: bool,
}

pub(super) fn write_from_env() -> Result<(), String> {
    let metadata = ReleaseMetadata {
        tag: required_env("RELEASE_TAG")?,
        commit: required_env("RELEASE_COMMIT")?,
        version: required_env("RELEASE_VERSION")?,
        validation_mode: parse_bool(&required_env("VALIDATION_MODE")?)?,
    };
    validate_metadata(&metadata)?;
    let output_dir = PathBuf::from(required_env("METADATA_OUTPUT_DIR")?);
    write_metadata(&output_dir, &metadata)?;
    println!(
        "Wrote build metadata for {} ({}).",
        metadata.tag, metadata.commit
    );
    Ok(())
}

pub(super) fn read_from_env() -> Result<(), String> {
    let input_dir = PathBuf::from(required_env("METADATA_INPUT_DIR")?);
    let expected_validation = parse_bool(&required_env("EXPECTED_VALIDATION_MODE")?)?;
    let metadata = read_metadata(&input_dir)?;
    if metadata.validation_mode != expected_validation {
        return Err(format!(
            "build metadata validation mode {} does not match CD validation mode {}",
            bool_text(metadata.validation_mode),
            bool_text(expected_validation)
        ));
    }

    println!(
        "Selected build metadata: {} ({}), version {}, validationMode={}",
        metadata.tag,
        metadata.commit,
        metadata.version,
        bool_text(metadata.validation_mode)
    );
    if let Ok(build_id) = env::var("BUILD_BUILDID") {
        if !build_id.is_empty() {
            println!(
                "##vso[build.updatebuildnumber]{}-cd-{build_id}",
                metadata.tag
            );
        }
    }
    azure_variable("releaseVersion", &metadata.version);
    azure_variable("releaseTag", &metadata.tag);
    azure_variable("releaseCommit", &metadata.commit);
    Ok(())
}

fn validate_metadata(metadata: &ReleaseMetadata) -> Result<(), String> {
    validate_stable_version(&metadata.version)?;
    validate_release_tag(&metadata.tag)?;
    if metadata.tag != format!("v{}", metadata.version) {
        return Err("release version must equal release tag without its v prefix".to_string());
    }
    validate_commit(&metadata.commit)
}

fn write_metadata(output_dir: &Path, metadata: &ReleaseMetadata) -> Result<(), String> {
    fs::create_dir_all(output_dir)
        .map_err(|error| format!("failed to create {}: {error}", output_dir.display()))?;
    write_line(&output_dir.join(METADATA_FILES[0].0), &metadata.tag)?;
    write_line(&output_dir.join(METADATA_FILES[1].0), &metadata.commit)?;
    write_line(&output_dir.join(METADATA_FILES[2].0), &metadata.version)?;
    write_line(
        &output_dir.join(METADATA_FILES[3].0),
        bool_text(metadata.validation_mode),
    )
}

fn read_metadata(input_dir: &Path) -> Result<ReleaseMetadata, String> {
    let metadata = ReleaseMetadata {
        tag: read_line(&input_dir.join(METADATA_FILES[0].0), METADATA_FILES[0].1)?,
        commit: read_line(&input_dir.join(METADATA_FILES[1].0), METADATA_FILES[1].1)?,
        version: read_line(&input_dir.join(METADATA_FILES[2].0), METADATA_FILES[2].1)?,
        validation_mode: parse_bool(&read_line(
            &input_dir.join(METADATA_FILES[3].0),
            METADATA_FILES[3].1,
        )?)?,
    };
    validate_metadata(&metadata)?;
    Ok(metadata)
}

fn write_line(path: &Path, value: &str) -> Result<(), String> {
    if value.contains(['\r', '\n']) {
        return Err(format!(
            "metadata value for {} contains a newline",
            path.display()
        ));
    }
    fs::write(path, format!("{value}\n"))
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn read_line(path: &Path, name: &str) -> Result<String, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {name} metadata {}: {error}", path.display()))?;
    let Some(value) = content.strip_suffix('\n') else {
        return Err(format!(
            "{name} metadata must contain exactly one newline-terminated line: {}",
            path.display()
        ));
    };
    if value.is_empty() || value.contains(['\r', '\n']) {
        return Err(format!(
            "{name} metadata must contain exactly one non-empty line: {}",
            path.display()
        ));
    }
    Ok(value.to_string())
}

fn azure_variable(name: &str, value: &str) {
    println!("##vso[task.setvariable variable={name}]{value}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata() -> ReleaseMetadata {
        ReleaseMetadata {
            tag: "v1.2.3".to_string(),
            commit: "0123456789abcdef0123456789abcdef01234567".to_string(),
            version: "1.2.3".to_string(),
            validation_mode: true,
        }
    }

    #[test]
    fn metadata_round_trips() {
        let directory = tempfile::TempDir::new().expect("temp directory should be created");
        let expected = metadata();

        write_metadata(directory.path(), &expected).expect("metadata should be written");
        let actual = read_metadata(directory.path()).expect("metadata should be read");

        assert_eq!(actual, expected);
    }

    #[test]
    fn metadata_requires_newline_terminated_single_line() {
        let directory = tempfile::TempDir::new().expect("temp directory should be created");
        let expected = metadata();
        write_metadata(directory.path(), &expected).expect("metadata should be written");
        fs::write(directory.path().join("release-tag.txt"), "v1.2.3")
            .expect("fixture should be written");

        let error = read_metadata(directory.path()).expect_err("metadata should be rejected");

        assert!(error.contains("newline-terminated"));
    }
}
