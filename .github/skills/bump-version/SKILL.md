---
name: bump-version
description: Bump WebUI to an input release version, run the full gate, and prepare a release PR with bucketed changes since the previous v-prefixed tag.
---

# Bump Version Release PR

Use this skill when preparing a WebUI release version bump PR.

## Inputs

Ask for the release version if the user did not provide one. The input must be the bare semver version, without a leading `v`:

```text
0.0.16
```

Use `v<version>` only when comparing or creating Git tags.

## Workflow

1. Make sure the working branch is not `main`. If needed, create a release branch named `<user>/bump-v<version>`.
2. Fetch tags so the release range is current:

   ```bash
   git fetch --tags --quiet
   ```

3. Run the version bump:

   ```bash
   cargo xtask version <version>
   ```

4. Run the full quality gate and fix any failures:

   ```bash
   cargo xtask check
   ```

5. Find the previous release tag. WebUI release tags are prefixed with `v`:

   ```bash
   previous_tag=$(git tag --list 'v[0-9]*' --sort=-v:refname | grep -v "^v<version>$" | head -n 1)
   ```

   Stop if no previous tag is found.

6. Collect commits from the previous release through the current base commit. Do this before committing the version bump so release notes do not include the bump commit:

   ```bash
   git --no-pager log --reverse --format='%s%x09%H' "${previous_tag}..HEAD"
   ```

7. Build the PR release notes by grouping all commits into user-facing buckets.
8. Commit the version bump with an imperative message:

   ```text
   chore: bump version to <version>

   Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>
   ```

9. Open or update the release PR titled:

   ```text
   chore: bump version to <version>
   ```

## Release notes bucketing

Use these buckets, omitting empty buckets:

| Bucket | Include |
|--------|---------|
| `Features` | `feat:` commits, feature PRs, new user-visible capabilities, demos that showcase new behavior |
| `Fixes` | `fix:` commits, bug fixes, regressions, correctness fixes |
| `Docs` | `docs:` commits, documentation-only PRs, policy/issue-template changes, and docs work from mixed PRs |
| `Maintenance` | Release-relevant chores, refactors, dependency or CI changes that do not fit the other buckets |

Prefer concise, user-facing summaries over raw commit subjects. Combine related commits into one bullet when they describe the same release-note item.

For each bullet, include the supporting PR title and number in parentheses:

```text
- summary sentence (PR title #number).
```

Do not include commit SHAs in the PR release notes. If a commit subject already contains a PR number, use it. Otherwise, use `gh` to inspect the commit or associated PR when possible.

## PR body template

```markdown
## Release

Bumps WebUI to `<version>`.

Previous release tag: `<previous_tag>`

## Changes since `<previous_tag>`

Features:

- <feature summary> (<PR title> #<number>).

Fixes:

- <fix summary> (<PR title> #<number>).

Docs:

- <docs summary> (<PR title> #<number>).

Maintenance:

- <maintenance summary> (<PR title> #<number>).

## Validation

- `cargo xtask check`
```

Remove empty buckets before opening or updating the PR.

## Example style

```markdown
Features:

- parser comment policy strips template/style comments while preserving legal comments, with CLI, Node, docs, and benchmark coverage (feat: strip template and style comments in parser and support legal comments #326).
- CSS module delivery now emits import-map data URI modules and the commerce demo defaults to module styles (Emit style modules via importmap + dataURI instead of <style type="module"> #325, Switch default styles to module for commerce demo #327).

Fixes:

- repeat-scope event arguments hydrate correctly for framework bindings, including strict argument handling (Fix event handler args in repeat scopes #317, fix: hydrate strict event arguments in repeat scopes #322).
- client binding lifecycle ordering preserves child updates across conditional and repeated DOM paths (fix: initialize child bindings before connection #329).
- compiled templates preserve raw style text instead of altering author-provided CSS content (fix: preserve raw style text in compiled templates #330).

Docs:

- issue forms, contribution/support policy, framework rendering docs, CLI docs, and integration guides were refreshed (chore: clarify contribution and support policy #321, chore: add GitHub issue forms #328, plus docs in fix: hydrate strict event arguments in repeat scopes #322/Emit style modules via importmap + dataURI instead of <style type="module"> #325/feat: strip template and style comments in parser and support legal comments #326/fix: initialize child bindings before connection #329).
```
