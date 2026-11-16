#!/usr/bin/env bash
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

# Production-only entry point. Tests run exclusively in isolated local clones.
set -euo pipefail

fail() {
  printf 'Release tagging failed: %s\n' "$*" >&2
  exit 1
}

[[ "${VALIDATION_MODE:-}" == false ]] ||
  fail "VALIDATION_MODE must explicitly be false; validation runs never create tags."
: "${RELEASE_TAG:?RELEASE_TAG is required}"
: "${RELEASE_COMMIT:?RELEASE_COMMIT is required}"
[[ "$RELEASE_TAG" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-hotfix\.[1-9][0-9]*)?$ ]] ||
  fail "unsupported release tag ${RELEASE_TAG}."
[[ "$RELEASE_COMMIT" =~ ^[0-9a-f]{40}$ ]] ||
  fail "RELEASE_COMMIT must be a full commit SHA."

git fetch --no-tags origin "$RELEASE_COMMIT"
resolved_commit=$(git rev-parse --verify "${RELEASE_COMMIT}^{commit}")
[[ "$resolved_commit" == "$RELEASE_COMMIT" ]] ||
  fail "${RELEASE_COMMIT} is not a commit."

tags=("$RELEASE_TAG")
if git cat-file -e "${RELEASE_COMMIT}:go/go.mod" 2>/dev/null; then
  module_path=$(git show "${RELEASE_COMMIT}:go/go.mod" |
    awk '$1 == "module" { value = $2; gsub(/^"|"$/, "", value); print value; count++ }
         END { if (count != 1) exit 1 }') ||
    fail "release go/go.mod must declare exactly one module."
  [[ "$module_path" == github.com/microsoft/webui/go/v3 ]] ||
    fail "release go/go.mod declares ${module_path}, expected github.com/microsoft/webui/go/v3."
  [[ "$RELEASE_TAG" == v3.* ]] ||
    fail "the /v3 Go module requires a v3 release tag."
  tags+=("go/${RELEASE_TAG}")
else
  printf 'Release commit has no go/go.mod; ensuring the historical root tag only.\n'
fi

# Return 2 only for a confirmed absent remote tag. Network/authentication errors
# must fail closed, not be interpreted as permission to create a tag.
remote_commit() {
  local tag="$1" status remote_ref
  if git ls-remote --exit-code origin "refs/tags/${tag}" "refs/tags/${tag}^{}" >/dev/null; then
    remote_ref="refs/azure-release-tags/${tag}"
    git fetch --no-tags --force origin "refs/tags/${tag}:${remote_ref}" >&2 || return 1
    git rev-parse --verify "${remote_ref}^{commit}"
  else
    status=$?
    [[ "$status" == 2 ]] && return 2
    return 1
  fi
}

# Preflight every tag before writing any. An existing correct root tag does not
# skip the independent Go tag; divergent tags are never retargeted.
missing=()
for tag in "${tags[@]}"; do
  if current=$(remote_commit "$tag"); then
    [[ "$current" == "$RELEASE_COMMIT" ]] ||
      fail "${tag} points to ${current}, not ${RELEASE_COMMIT}."
    printf '%s already points to %s.\n' "$tag" "$RELEASE_COMMIT"
  else
    status=$?
    [[ "$status" == 2 ]] || fail "unable to query ${tag} on origin."
    missing+=("$tag")
  fi
done

# Bash 3 (macOS) treats empty arrays as unbound under set -u.
if [[ "${#missing[@]}" == 0 ]]; then
  exit 0
fi
for tag in "${missing[@]}"; do
  if git show-ref --verify --quiet "refs/tags/${tag}"; then
    local_commit=$(git rev-parse --verify "refs/tags/${tag}^{commit}")
    [[ "$local_commit" == "$RELEASE_COMMIT" ]] ||
      fail "local ${tag} points to ${local_commit}, not ${RELEASE_COMMIT}."
  else
    git -c user.name="Azure Pipelines" -c user.email="azure-pipelines@microsoft.com" \
      -c tag.gpgSign=false tag -a "$tag" "$RELEASE_COMMIT" -m "Release ${tag}"
  fi
  if git push origin "refs/tags/${tag}:refs/tags/${tag}"; then
    printf 'Created %s at %s.\n' "$tag" "$RELEASE_COMMIT"
    continue
  fi
  if raced_commit=$(remote_commit "$tag"); then
    [[ "$raced_commit" == "$RELEASE_COMMIT" ]] ||
      fail "concurrent ${tag} points to ${raced_commit}, not ${RELEASE_COMMIT}."
    printf 'Concurrent release created %s at the expected commit.\n' "$tag"
  else
    fail "unable to create or revalidate ${tag} on origin."
  fi
done
