# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

# Run inside portable-linux.Dockerfile with the checkout at /io, staged output
# at /out, and the host's compressed crate-download cache at /cargo-cache.
set -euo pipefail

: "${TARGET_TRIPLE:?TARGET_TRIPLE is required}"
: "${ARTIFACT_NAME:?ARTIFACT_NAME is required}"
: "${RELEASE_VERSION:?RELEASE_VERSION is required}"

source /etc/os-release
if [[ "$ID" != ubuntu || "$VERSION_ID" != 22.04 || "$(uname -m)" != x86_64 ]]; then
  echo "Portable release builds require an x86_64 Ubuntu 22.04 container" >&2
  exit 1
fi
case "$TARGET_TRIPLE" in
  x86_64-unknown-linux-gnu) platform=linux-x64 ;;
  aarch64-unknown-linux-gnu)
    platform=linux-arm64
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
    export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
    export CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++
    ;;
  *)
    echo "Unsupported portable Linux target: $TARGET_TRIPLE" >&2
    exit 1
    ;;
esac

# Run as the agent's UID. Keep toolchains, extracted crates, and compiled
# objects inside the disposable container, not in the checkout or host cache.
export HOME=/tmp/webui-home
export CARGO_HOME=/tmp/webui-cargo
export RUSTUP_HOME=/tmp/webui-rustup
export CARGO_TARGET_DIR=/tmp/webui-target
export CARGO_INCREMENTAL=0
mkdir -p "$HOME" "$CARGO_HOME/registry"
ln -s /cargo-cache "$CARGO_HOME/registry/cache"

rustup_version=1.29.0
rustup_sha256=4acc9acc76d5079515b46346a485974457b5a79893cfb01112423c89aeb5aa10
rust_toolchain=1.99.1
rustup_init="/tmp/rustup-init-${rustup_version}-x86_64-unknown-linux-gnu"
trap 'rm -f "$rustup_init"' EXIT
curl --proto "=https" --tlsv1.2 --fail --silent --show-error \
  --output "$rustup_init" \
  "https://static.rust-lang.org/rustup/archive/${rustup_version}/x86_64-unknown-linux-gnu/rustup-init"
printf "%s  %s\n" "$rustup_sha256" "$rustup_init" | sha256sum --check --strict -
chmod +x "$rustup_init"
"$rustup_init" -y --profile minimal --default-toolchain none --no-modify-path
rm -f "$rustup_init"
trap - EXIT
export PATH="$CARGO_HOME/bin:$PATH"
rustup toolchain install "$rust_toolchain" --profile minimal --target "$TARGET_TRIPLE"
rustc +"$rust_toolchain" --version
go version

output="/out/$ARTIFACT_NAME"
cargo +"$rust_toolchain" xtask publish-build \
  --target "$TARGET_TRIPLE" \
  --profile release \
  --portable-only \
  --output "$output"

# Audit each exact staged Cargo artifact, including the separate compiler addon.
for file in \
  "$output/packages/webui-$platform/bin/webui" \
  "$output/packages/webui-press-$platform/bin/webui-press" \
  "$output/packages/webui-$platform/webui.node" \
  "$output/packages/webui-$platform/webui-build.node" \
  "$output/dotnet/runtimes/$platform/native/libwebui_ffi.so"; do
  required=$(readelf --version-info --wide "$file" |
    grep -oE 'GLIBC_[0-9.]+' | sort -Vu | tail -n 1)
  if [[ $(printf '%s\n' GLIBC_2.35 "$required" | sort -V | tail -n 1) != GLIBC_2.35 ]]; then
    echo "$file requires $required; expected glibc 2.35 or older" >&2
    exit 1
  fi
done

# The producer/HTTPS helper runs on x64; it never executes an ARM64 artifact.
cargo +"$rust_toolchain" xtask toolchain-stage \
  --target "$TARGET_TRIPLE" \
  --cli "$CARGO_TARGET_DIR/$TARGET_TRIPLE/release/webui" \
  --acquire-checker
toolchain="publish/toolchains/webui-toolchain-${RELEASE_VERSION}-${TARGET_TRIPLE}.tar.gz"
mkdir -p "$output/publish/toolchains"
cp "$toolchain" "$toolchain.sha256" "$output/publish/toolchains/"
