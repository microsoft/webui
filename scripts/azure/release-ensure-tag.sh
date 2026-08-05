#!/usr/bin/env bash
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

# Create the annotated Git tag consumed by the GitHub Release task.
set -euo pipefail

: "${RELEASE_TAG:?RELEASE_TAG is required}"
: "${RELEASE_COMMIT:?RELEASE_COMMIT is required}"

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

is_commit_id() {
  local value="$1"
  [[ ${#value} -eq 40 && -z "${value//[0-9a-f]/}" ]]
}

if [[ "$RELEASE_TAG" == *$'\n'* || "${RELEASE_TAG#v}" == "$RELEASE_TAG" ]] ||
  ! is_stable_version "${RELEASE_TAG#v}"; then
  echo "RELEASE_TAG must exactly match vMAJOR.MINOR.PATCH." >&2
  exit 1
fi
if ! is_commit_id "$RELEASE_COMMIT"; then
  echo "RELEASE_COMMIT must be a 40-character lowercase commit ID." >&2
  exit 1
fi

remote_tag_commit() {
  local output status commit

  set +e
  output=$(git ls-remote --exit-code origin \
    "refs/tags/${RELEASE_TAG}" "refs/tags/${RELEASE_TAG}^{}" 2>&1)
  status=$?
  set -e
  if (( status == 2 )); then
    return 1
  fi
  if (( status != 0 )); then
    echo "Unable to query ${RELEASE_TAG} from origin: ${output}" >&2
    return 2
  fi

  commit=$(printf '%s\n' "$output" |
    awk -v ref="refs/tags/${RELEASE_TAG}^{}" '$2 == ref { print $1; exit }')
  if [[ -z "$commit" ]]; then
    commit=$(printf '%s\n' "$output" |
      awk -v ref="refs/tags/${RELEASE_TAG}" '$2 == ref { print $1; exit }')
  fi
  if ! is_commit_id "$commit"; then
    echo "Remote tag ${RELEASE_TAG} did not peel to a valid commit ID." >&2
    return 2
  fi
  printf '%s\n' "$commit"
}

git fetch origin --tags
if ! git cat-file -e "${RELEASE_COMMIT}^{commit}" 2>/dev/null; then
  git fetch --no-tags origin "$RELEASE_COMMIT"
fi
if ! git cat-file -e "${RELEASE_COMMIT}^{commit}" 2>/dev/null; then
  echo "RELEASE_COMMIT does not identify a commit available from origin." >&2
  exit 1
fi

if existing_commit=$(remote_tag_commit); then
  if [[ "$existing_commit" != "$RELEASE_COMMIT" ]]; then
    echo "Remote tag ${RELEASE_TAG} points to ${existing_commit}, not ${RELEASE_COMMIT}." >&2
    exit 1
  fi
  echo "Remote tag ${RELEASE_TAG} already points to ${RELEASE_COMMIT}."
  exit 0
else
  status=$?
  if (( status != 1 )); then
    exit 1
  fi
fi

git config user.name "Azure Pipelines"
git config user.email "azure-pipelines@microsoft.com"
if git show-ref --verify --quiet "refs/tags/${RELEASE_TAG}"; then
  git tag -d "$RELEASE_TAG"
fi
git tag -a "$RELEASE_TAG" "$RELEASE_COMMIT" -m "Release ${RELEASE_TAG}"

if git push origin "refs/tags/${RELEASE_TAG}"; then
  echo "Created annotated release tag ${RELEASE_TAG} at ${RELEASE_COMMIT}."
  exit 0
fi

echo "Tag push failed; checking whether another release run created the same tag." >&2
git fetch --force origin \
  "refs/tags/${RELEASE_TAG}:refs/azure-release-tags/${RELEASE_TAG}"
raced_commit=$(git rev-parse "refs/azure-release-tags/${RELEASE_TAG}^{}")
if ! is_commit_id "$raced_commit"; then
  echo "Concurrent remote tag ${RELEASE_TAG} did not peel to a valid commit ID." >&2
  exit 1
fi
if [[ "$raced_commit" != "$RELEASE_COMMIT" ]]; then
  echo "Concurrent remote tag ${RELEASE_TAG} points to ${raced_commit}, not ${RELEASE_COMMIT}." >&2
  exit 1
fi
echo "Concurrent release run created ${RELEASE_TAG} at the expected commit."
