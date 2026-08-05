#!/usr/bin/env bash
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

set -euo pipefail

: "${BUILD_ARTIFACT_STAGING_DIRECTORY:?BUILD_ARTIFACT_STAGING_DIRECTORY is required}"

sudo apt-get update -q
sudo apt-get install -y -q \
  protobuf-compiler \
  gcc-aarch64-linux-gnu \
  g++-aarch64-linux-gnu
rustup toolchain install 1.93 --profile minimal \
  --target x86_64-unknown-linux-gnu,aarch64-unknown-linux-gnu
rustup default 1.93
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc

cargo build --release \
  --target x86_64-unknown-linux-gnu \
  -p microsoft-webui-cli \
  -p microsoft-webui-ffi \
  -p microsoft-webui-node
cargo build --release \
  --target aarch64-unknown-linux-gnu \
  -p microsoft-webui-cli \
  -p microsoft-webui-ffi \
  -p microsoft-webui-node
cargo xtask publish-stage --target all --profile release --native-only

stage_dir="${BUILD_ARTIFACT_STAGING_DIRECTORY}/stage-linux"
mkdir -p "$stage_dir/publish" "$stage_dir/packages" "$stage_dir/dotnet/runtimes"
cp -R publish/native "$stage_dir/publish/"
cp -R packages/webui-linux-x64 packages/webui-linux-arm64 "$stage_dir/packages/"
cp -R dotnet/runtimes/linux-x64 dotnet/runtimes/linux-arm64 "$stage_dir/dotnet/runtimes/"
