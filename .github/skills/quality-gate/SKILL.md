---
name: quality-gate
description: Run before every commit or push: formatting, linting, tests, dependency audits, and builds.
---

# Quality Gate Workflow

Use this skill whenever work changes code, tests, dependencies, or docs in this repository.

## Required gate

Before any commit, run:

```bash
cargo xtask check
```

This runs, in order: `fmt → clippy → deny → test → build → doc`.

Missing Rust tools (`clippy`, `rustfmt`, `cargo-deny`, `wasm-pack`, `wasm32-unknown-unknown` target) are **auto-installed** on first run — no manual setup needed.

Work is not complete until it passes cleanly.

## Fast iteration sequence

When iterating locally, use this order:

1. Targeted crate checks first (for faster feedback):
   - `cargo test -p <crate>`
2. Then full gate:
   - `cargo xtask check`

## Expectations

- Fix reported issues rather than suppressing them.
- Do not merge or commit with a failing gate.
- Keep fixes minimal and scoped to the task.
