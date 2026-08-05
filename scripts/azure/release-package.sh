#!/usr/bin/env bash
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

# Assemble merged native outputs into the artifact sets consumed by the release pipeline.
set -euo pipefail
shopt -s nullglob

: "${PACKAGE_OUTPUT_ROOT:?PACKAGE_OUTPUT_ROOT is required}"
: "${RELEASE_SOURCE_DIR:?RELEASE_SOURCE_DIR is required}"

cd "$RELEASE_SOURCE_DIR"

sudo apt-get update -q
sudo apt-get install -y -q protobuf-compiler
rustup toolchain install 1.93 --profile minimal
rustup default 1.93
corepack enable
pnpm install --frozen-lockfile
cargo xtask publish-stage --pack-only --profile release

npm_artifacts=(publish/npm/*.tgz)
crate_artifacts=(publish/crates/*.crate)
nuget_artifacts=(publish/nuget/*.nupkg)
nuget_symbol_artifacts=(publish/nuget/*.snupkg)
standalone_artifacts=(publish/standalone/*)
if (( ${#npm_artifacts[@]} == 0 )); then
  echo "No npm artifacts were rebuilt." >&2
  exit 1
fi
if (( ${#crate_artifacts[@]} == 0 )); then
  echo "No crate artifacts were rebuilt." >&2
  exit 1
fi
if (( ${#nuget_artifacts[@]} == 0 )); then
  echo "No NuGet package artifacts were rebuilt." >&2
  exit 1
fi
if (( ${#nuget_symbol_artifacts[@]} == 0 )); then
  echo "No NuGet symbol package artifacts were rebuilt." >&2
  exit 1
fi
if (( ${#standalone_artifacts[@]} != 20 )); then
  echo "Expected 20 standalone release assets, found ${#standalone_artifacts[@]}." >&2
  exit 1
fi

npm_output="${PACKAGE_OUTPUT_ROOT}/publish_artifacts_npm"
crate_output="${PACKAGE_OUTPUT_ROOT}/publish_artifacts_crates"
nuget_output="${PACKAGE_OUTPUT_ROOT}/publish_artifacts_nuget"
standalone_output="${PACKAGE_OUTPUT_ROOT}/publish_artifacts_standalone"
rm -rf -- "$npm_output" "$crate_output" "$nuget_output" "$standalone_output"
mkdir -p "$npm_output" "$crate_output" "$nuget_output" "$standalone_output"
cp "${npm_artifacts[@]}" "$npm_output/"
cp "${crate_artifacts[@]}" "$crate_output/"
cp "${nuget_artifacts[@]}" "$nuget_output/"
cp "${nuget_symbol_artifacts[@]}" "$nuget_output/"
cp "${standalone_artifacts[@]}" "$standalone_output/"

echo "npm artifacts:"
ls "$npm_output/"
echo "crate artifacts:"
ls "$crate_output/"
echo "NuGet artifacts:"
ls "$nuget_output/"
echo "Standalone release assets:"
ls "$standalone_output/"
