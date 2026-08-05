#!/usr/bin/env bash
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

set -euo pipefail

: "${BUILD_ARTIFACT_STAGING_DIRECTORY:?BUILD_ARTIFACT_STAGING_DIRECTORY is required}"

if ! command -v protoc >/dev/null 2>&1; then
  brew install protobuf
fi
rustup toolchain install 1.93 --profile minimal \
  --target aarch64-apple-darwin,x86_64-apple-darwin
rustup default 1.93

cargo build --release \
  --target aarch64-apple-darwin \
  -p microsoft-webui-cli \
  -p microsoft-webui-ffi \
  -p microsoft-webui-node
cargo build --release \
  --target x86_64-apple-darwin \
  -p microsoft-webui-cli \
  -p microsoft-webui-ffi \
  -p microsoft-webui-node
cargo xtask publish-stage --target all --profile release --native-only

stage_dir="${BUILD_ARTIFACT_STAGING_DIRECTORY}/stage-macos"
mkdir -p "$stage_dir/publish" "$stage_dir/packages" "$stage_dir/dotnet/runtimes"
cp -R publish/native "$stage_dir/publish/"
cp -R packages/webui-darwin-arm64 packages/webui-darwin-x64 "$stage_dir/packages/"
cp -R dotnet/runtimes/osx-arm64 dotnet/runtimes/osx-x64 "$stage_dir/dotnet/runtimes/"
