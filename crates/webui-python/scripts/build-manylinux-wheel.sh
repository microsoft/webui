# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.
#
# Build one manylinux_2_17 abi3 wheel inside a messense/manylinux2014-cross
# image. The image supplies GCC, the target Python headers, and the sysroot,
# but no Rust, so this installs a pinned toolchain from a checksum-verified
# rustup-init rather than the mutable https://sh.rustup.rs bootstrap.
#
# Usage: build-manylinux-wheel.sh <target-triple> [export-root]
#
# The wheel is always written to publish/python/ in the repository. Passing an
# export root additionally copies it to <export-root>/publish/python/.

set -euo pipefail

target="${1:?target triple is required}"
output="${2:-}"

rustup_version=1.29.0
rustup_sha256=4acc9acc76d5079515b46346a485974457b5a79893cfb01112423c89aeb5aa10
rust_toolchain=1.93
maturin_version=1.14.1

# Both cross images run as amd64; a mismatch means the wrong image was pulled.
if [[ "$(uname -m)" != x86_64 ]]; then
  echo "Expected x86_64 manylinux build host, found $(uname -m)" >&2
  exit 1
fi

rustup_init="target/rustup-init-${rustup_version}-x86_64-unknown-linux-gnu"
mkdir -p target
trap 'rm -f "$rustup_init"' EXIT

curl --proto "=https" --tlsv1.2 --fail --silent --show-error \
  --output "$rustup_init" \
  "https://static.rust-lang.org/rustup/archive/${rustup_version}/x86_64-unknown-linux-gnu/rustup-init"
printf "%s  %s\n" "$rustup_sha256" "$rustup_init" | sha256sum --check --strict -
chmod +x "$rustup_init"
"$rustup_init" -y --profile minimal --default-toolchain none --no-modify-path
rm -f "$rustup_init"
trap - EXIT

export PATH="$HOME/.cargo/bin:$PATH"
export RUSTUP_TOOLCHAIN="$rust_toolchain"

rustup toolchain install "$rust_toolchain" --profile minimal --target "$target"
rustc --version
cargo --version

python3.11 -m pip install --upgrade "maturin==${maturin_version}"

# Wheel build policy lives in xtask, exactly like every other release artifact.
# This script only provides the old-glibc environment that policy runs inside.
#
# The cross image pre-sets CARGO_BUILD_TARGET so cargo cross-compiles by
# default. xtask is a host build tool, so clear it here: leaving it set makes
# cargo build xtask for the target and then fail trying to execute it.
build_args=(--target "$target" --python-only)
if [[ -n "$output" ]]; then
  build_args+=(--output "$output")
fi
WEBUI_PYTHON=python3.11 env -u CARGO_BUILD_TARGET cargo xtask publish-build "${build_args[@]}"
