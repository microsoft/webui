# Pull Request Conventions

## PR title format

PR titles must use a [Conventional Commits](https://www.conventionalcommits.org/) prefix:

| Prefix | When to use | Example |
|--------|-------------|---------|
| `feat:` | New feature or capability | `feat: add HTTP/2 support to hyper example` |
| `fix:` | Bug fix | `fix: render missing signals as empty` |
| `chore:` | Maintenance, refactoring, CI, docs, dependencies | `chore: move shared files to examples/app` |

The prefix is **lowercase**, followed by a colon and a space, then a short imperative description.

## Linking PRs to issues

When a PR is meant to close a GitHub issue, include the keyword `Closes` followed by the issue number in the PR description body (not the title):

```
Closes #42
```

For issues in a different repository:

```
Closes owner/repo#42
```

For multiple issues:

```
Closes #42, closes #43
```

> **Note:** Issue-linking keywords only work when the PR targets the repository's default branch. See [GitHub docs: linking a pull request to an issue](https://docs.github.com/en/issues/tracking-your-work-with-issues/using-issues/linking-a-pull-request-to-an-issue#linking-a-pull-request-to-an-issue-using-a-keyword) for the full reference.
