#!/usr/bin/env bash
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

# Resolve the workspace release and reject versions that were already tagged.
set -euo pipefail

log_error() {
  echo "##vso[task.logissue type=error]$*" >&2
}

is_stable_version() {
  local value="$1"
  local major minor patch extra

  [[ "$value" != *$'\n'* ]] || return 1
  IFS=. read -r major minor patch extra <<<"$value"
  [[ -n "$major" && -n "$minor" && -n "$patch" && -z "${extra:-}" ]] || return 1
  [[ -z "${major//[0-9]/}" && -z "${minor//[0-9]/}" && -z "${patch//[0-9]/}" ]] ||
    return 1
  [[ "$major" == 0 || "$major" != 0* ]] &&
    [[ "$minor" == 0 || "$minor" != 0* ]] &&
    [[ "$patch" == 0 || "$patch" != 0* ]]
}

parse_workspace_version() {
  local version

  if ! version=$(awk '
    BEGIN {
      in_workspace_package = 0
      version_count = 0
      invalid = 0
    }
    /^[[:space:]]*\[/ {
      in_workspace_package = ($0 ~ /^[[:space:]]*\[workspace[.]package\][[:space:]]*(#.*)?$/)
      next
    }
    in_workspace_package && /^[[:space:]]*version[[:space:]]*=/ {
      value = $0
      sub(/^[[:space:]]*version[[:space:]]*=[[:space:]]*/, "", value)
      if (value !~ /^"[0-9]+[.][0-9]+[.][0-9]+"[[:space:]]*(#.*)?$/) {
        invalid = 1
        next
      }
      sub(/^"/, "", value)
      sub(/"[[:space:]]*(#.*)?$/, "", value)
      version = value
      version_count++
    }
    END {
      if (invalid || version_count != 1) {
        exit 1
      }
      print version
    }
  '); then
    log_error "Cargo.toml must contain exactly one canonical stable version in [workspace.package]."
    return 1
  fi

  if ! is_stable_version "$version"; then
    log_error "Workspace version must use stable SemVer MAJOR.MINOR.PATCH with ASCII digits only."
    return 1
  fi
  printf '%s\n' "$version"
}

is_commit_id() {
  local value="$1"
  [[ ${#value} -eq 40 && -z "${value//[0-9a-f]/}" ]]
}

resolve_remote_tag() {
  local tag="$1"
  local output status commit

  set +e
  output=$(git ls-remote --exit-code origin \
    "refs/tags/${tag}" "refs/tags/${tag}^{}" 2>&1)
  status=$?
  set -e
  if (( status == 2 )); then
    return 1
  fi
  if (( status != 0 )); then
    log_error "Unable to query tag ${tag} from checkout repository origin: ${output}"
    return 2
  fi

  commit=$(printf '%s\n' "$output" |
    awk -v ref="refs/tags/${tag}^{}" '$2 == ref { print $1; exit }')
  if [[ -z "$commit" ]]; then
    commit=$(printf '%s\n' "$output" |
      awk -v ref="refs/tags/${tag}" '$2 == ref { print $1; exit }')
  fi
  if ! is_commit_id "$commit"; then
    log_error "Remote tag ${tag} did not resolve to a 40-character lowercase commit ID."
    return 2
  fi
  printf '%s\n' "$commit"
}

case "${BUILD_SOURCEBRANCH:-}" in
  refs/heads/*) ;;
  *)
    log_error "BUILD_SOURCEBRANCH must identify a branch as refs/heads/*."
    exit 1
    ;;
esac
release_commit="${BUILD_SOURCEVERSION:-}"
if ! is_commit_id "$release_commit"; then
  log_error "BUILD_SOURCEVERSION must be a 40-character lowercase commit ID."
  exit 1
fi
if ! release_version=$(parse_workspace_version <Cargo.toml); then
  exit 1
fi
release_tag="v${release_version}"
allow_existing_release="${ALLOW_EXISTING_RELEASE:-false}"
case "$allow_existing_release" in
  true | false) ;;
  *)
    log_error "ALLOW_EXISTING_RELEASE must be true or false."
    exit 1
    ;;
esac
if remote_commit=$(resolve_remote_tag "$release_tag"); then
  if [[ "$allow_existing_release" == true ]]; then
    should_build=true
    echo "Remote tag ${release_tag} already exists at ${remote_commit}; rebuilding artifacts for validation."
  else
    should_build=false
    echo "Remote tag ${release_tag} already exists at ${remote_commit}; skipping release build."
  fi
else
  status=$?
  if (( status != 1 )); then
    exit 1
  fi
  should_build=true
fi

echo "Resolved release: ${release_tag} (${release_commit}); shouldBuild=${should_build}"
if [[ -n "${BUILD_BUILDID:-}" ]]; then
  echo "##vso[build.updatebuildnumber]${release_tag}-build-${BUILD_BUILDID}"
fi
echo "##vso[task.setvariable variable=releaseVersion;isOutput=true]${release_version}"
echo "##vso[task.setvariable variable=releaseTag;isOutput=true]${release_tag}"
echo "##vso[task.setvariable variable=releaseCommit;isOutput=true]${release_commit}"
echo "##vso[task.setvariable variable=shouldBuild;isOutput=true]${should_build}"
