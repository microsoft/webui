#!/usr/bin/env bash
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

set -euo pipefail

: "${BUILD_SOURCES_DIRECTORY:?BUILD_SOURCES_DIRECTORY is required}"
: "${PIPELINE_WORKSPACE:?PIPELINE_WORKSPACE is required}"

for artifact_mapping in \
  unsigned_npm_packages:publish_artifacts_npm \
  unsigned_crate_packages:publish_artifacts_crates \
  nuget_signing_input:publish_artifacts_nuget \
  standalone_release_assets:publish_artifacts_standalone; do
  artifact_name=${artifact_mapping%%:*}
  publish_dir=${artifact_mapping#*:}
  source_dir="${PIPELINE_WORKSPACE}/releaseBuild/${artifact_name}"
  destination_dir="${BUILD_SOURCES_DIRECTORY}/${publish_dir}"

  if [[ ! -d "$source_dir" ]]; then
    echo "Downloaded build artifact directory is missing: ${source_dir}" >&2
    echo "Select a Web UI - CD Build run that assembled all required artifacts." >&2
    exit 1
  fi
  if [[ -z "$(find "$source_dir" -mindepth 1 -maxdepth 1 -print -quit)" ]]; then
    echo "Downloaded build artifact directory is empty: ${source_dir}" >&2
    echo "Select a successful Web UI - CD Build run with assembled release artifacts." >&2
    exit 1
  fi

  rm -rf -- "$destination_dir"
  mkdir -p "$destination_dir"
  cp -R "$source_dir"/. "$destination_dir"/
done

echo "Staged all release package sets and standalone assets."
