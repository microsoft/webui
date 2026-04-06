---
name: quality-gate
description: "Run before every commit or push: formatting, linting, tests, dependency audits, builds, and docs."
---

# Quality Gate

Run this before every commit. Work is not done until it passes.

## Gate command

```bash
cargo xtask check
```

This runs, in order: `license-headers -> fmt -> clippy -> deny -> test -> build`.

Missing Rust tools (`clippy`, `rustfmt`, `cargo-deny`, `wasm-pack`, `wasm32-unknown-unknown` target) are auto-installed on first run.

## Fast iteration

When iterating locally, use targeted crate checks for faster feedback:

```bash
cargo test -p microsoft-webui-handler   # test one crate
cargo xtask check                        # then full gate before commit
```

## Docs gate

If the change touches user-visible behavior, also run:

```bash
cd docs && pnpm build
```

This catches broken links, VitePress syntax errors, and missing pages. See the `docs-sync` skill for which docs files to update based on what changed.

## Code quality

The gate catches formatting, lint, and test failures. For deeper code quality checks (correctness, performance, concurrency, API design, architecture), apply the `code-review` skill.

## Expectations

- Fix reported issues rather than suppressing them.
- Do not commit with a failing gate.
- Keep fixes minimal and scoped to the task.
