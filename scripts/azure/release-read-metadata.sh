#!/usr/bin/env bash
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

# Restore the release identity selected by Web UI - CD Build.
set -euo pipefail

: "${METADATA_INPUT_DIR:?METADATA_INPUT_DIR is required}"
: "${EXPECTED_VALIDATION_MODE:?EXPECTED_VALIDATION_MODE is required}"

read_metadata_file() {
  local path=$1
  local name=$2
  local value

  if [[ ! -f "$path" ]]; then
    echo "Required build metadata file is missing: ${path}" >&2
    return 1
  fi
  if [[ $(wc -l < "$path") -ne 1 ]]; then
    echo "Build metadata file must contain exactly one newline-terminated line: ${path}" >&2
    return 1
  fi

  value=$(cat "$path")
  if [[ -z "$value" || "$value" == *$'\n'* ]]; then
    echo "Build metadata file contains an invalid ${name}: ${path}" >&2
    return 1
  fi
  printf '%s' "$value"
}

release_tag=$(read_metadata_file "$METADATA_INPUT_DIR/release-tag.txt" "release tag")
release_commit=$(read_metadata_file "$METADATA_INPUT_DIR/release-commit.txt" "release commit")
release_version=$(read_metadata_file "$METADATA_INPUT_DIR/release-version.txt" "release version")
validation_mode=$(read_metadata_file "$METADATA_INPUT_DIR/validation-mode.txt" "validation mode")

if [[ ! "$release_tag" =~ ^v.+$ ]]; then
  echo "Build metadata release tag must be v-prefixed." >&2
  exit 1
fi
if [[ ! "$release_commit" =~ ^[0-9a-f]{40}$ ]]; then
  echo "Build metadata release commit must be 40-character lowercase hexadecimal." >&2
  exit 1
fi
if [[ "$release_version" != "${release_tag#v}" ]]; then
  echo "Build metadata release version must equal the release tag without its v prefix." >&2
  exit 1
fi
if [[ "$validation_mode" != true && "$validation_mode" != false ]]; then
  echo "Build metadata validation mode must be true or false." >&2
  exit 1
fi
if [[ "$validation_mode" != "$EXPECTED_VALIDATION_MODE" ]]; then
  echo "Build metadata validation mode ${validation_mode} does not match CD validation mode ${EXPECTED_VALIDATION_MODE}." >&2
  exit 1
fi

echo "Selected build metadata: ${release_tag} (${release_commit}), version ${release_version}, validationMode=${validation_mode}"
if [[ -n "${BUILD_BUILDID:-}" ]]; then
  echo "##vso[build.updatebuildnumber]${release_tag}-cd-${BUILD_BUILDID}"
fi
echo "##vso[task.setvariable variable=releaseVersion]${release_version}"
echo "##vso[task.setvariable variable=releaseTag]${release_tag}"
echo "##vso[task.setvariable variable=releaseCommit]${release_commit}"
