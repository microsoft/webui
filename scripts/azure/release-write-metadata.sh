#!/usr/bin/env bash
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

# Persist the selected release identity across the pipeline resource boundary.
set -euo pipefail

: "${METADATA_OUTPUT_DIR:?METADATA_OUTPUT_DIR is required}"
: "${RELEASE_TAG:?RELEASE_TAG is required}"
: "${RELEASE_COMMIT:?RELEASE_COMMIT is required}"
: "${RELEASE_VERSION:?RELEASE_VERSION is required}"
: "${VALIDATION_MODE:?VALIDATION_MODE is required}"

if [[ "$RELEASE_TAG" == *$'\n'* || ! "$RELEASE_TAG" =~ ^v.+$ ]]; then
  echo "RELEASE_TAG must be a v-prefixed release tag." >&2
  exit 1
fi
if [[ ! "$RELEASE_COMMIT" =~ ^[0-9a-f]{40}$ ]]; then
  echo "RELEASE_COMMIT must be a 40-character lowercase hexadecimal commit." >&2
  exit 1
fi
if [[ "$RELEASE_VERSION" == *$'\n'* || "$RELEASE_VERSION" != "${RELEASE_TAG#v}" ]]; then
  echo "RELEASE_VERSION must equal RELEASE_TAG without its v prefix." >&2
  exit 1
fi
if [[ "$VALIDATION_MODE" != true && "$VALIDATION_MODE" != false ]]; then
  echo "VALIDATION_MODE must be true or false." >&2
  exit 1
fi

mkdir -p "$METADATA_OUTPUT_DIR"
printf '%s\n' "$RELEASE_TAG" > "$METADATA_OUTPUT_DIR/release-tag.txt"
printf '%s\n' "$RELEASE_COMMIT" > "$METADATA_OUTPUT_DIR/release-commit.txt"
printf '%s\n' "$RELEASE_VERSION" > "$METADATA_OUTPUT_DIR/release-version.txt"
printf '%s\n' "$VALIDATION_MODE" > "$METADATA_OUTPUT_DIR/validation-mode.txt"

echo "Wrote build metadata for ${RELEASE_TAG} (${RELEASE_COMMIT})."
