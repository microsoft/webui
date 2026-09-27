#!/usr/bin/env bash
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

# Source this script in the same shell that invokes cargo publish-build.
set -euo pipefail

target="${1:?pass a Linux Rust target triple}"
if [[ "$target" == x86_64-unknown-linux-gnu ]]; then
  sudo apt-get update -q
  sudo apt-get install -y -q protobuf-compiler pkg-config libgtk-4-dev libwebkitgtk-6.0-dev
elif [[ "$target" == aarch64-unknown-linux-gnu ]]; then
  source /etc/os-release
  if [[ "$ID" != ubuntu || -z "${VERSION_CODENAME:-}" ]]; then
    echo "Linux ARM64 desktop cross-build requires an Ubuntu agent with a release codename" >&2
    return 1
  fi
  for package in libgtk-4-dev libwebkitgtk-6.0-dev libpango1.0-dev; do
    if dpkg-query -W -f='${Status}' "${package}:amd64" 2>/dev/null | grep -q 'install ok installed'; then
      echo "Linux ARM64 desktop cross-build cannot co-install ${package}:amd64 with target development packages" >&2
      return 1
    fi
  done
  sudo dpkg --add-architecture arm64
  sources=$(mktemp --suffix=.sources)
  cat > "$sources" <<EOF
Types: deb
URIs: http://archive.ubuntu.com/ubuntu
Suites: $VERSION_CODENAME $VERSION_CODENAME-updates
Components: main restricted universe multiverse
Architectures: amd64
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg

Types: deb
URIs: http://security.ubuntu.com/ubuntu
Suites: $VERSION_CODENAME-security
Components: main restricted universe multiverse
Architectures: amd64
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg

Types: deb
URIs: http://ports.ubuntu.com/ubuntu-ports
Suites: $VERSION_CODENAME $VERSION_CODENAME-updates $VERSION_CODENAME-security
Components: main restricted universe multiverse
Architectures: arm64
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg
EOF
  apt_options=(-o "Dir::Etc::sourcelist=$sources" -o "Dir::Etc::sourceparts=-")
  sudo apt-get "${apt_options[@]}" update -q
  sudo apt-get "${apt_options[@]}" install -y -q \
    protobuf-compiler pkg-config gcc-aarch64-linux-gnu g++-aarch64-linux-gnu \
    libgtk-4-dev:arm64 libwebkitgtk-6.0-dev:arm64
  rm "$sources"
  export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
  export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
  export CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++
  export PKG_CONFIG_ALLOW_CROSS=1
  unset PKG_CONFIG_PATH PKG_CONFIG_SYSROOT_DIR
  export PKG_CONFIG_LIBDIR=/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/share/pkgconfig
  for pc in gtk4 webkitgtk-6.0; do
    pcdir="$(pkg-config --variable=pcfiledir "$pc")"
    if [[ "$pcdir" != /usr/lib/aarch64-linux-gnu/pkgconfig ]]; then
      echo "$pc resolved from unexpected directory: $pcdir" >&2
      return 1
    fi
  done
else
  echo "Unsupported Linux desktop target: $target" >&2
  return 1
fi
