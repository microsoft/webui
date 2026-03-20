---
name: quality-gate
description: "Run before every commit or push: formatting, linting, tests, dependency audits, and builds."
---

# Quality Gate Workflow

Use this skill whenever work changes code, tests, dependencies, or docs in this repository.

## Required gate

Before any commit, run:

```bash
cargo xtask check
```

This runs, in order: `license-headers → fmt → clippy → deny → test → build`.

Missing Rust tools (`clippy`, `rustfmt`, `cargo-deny`, `wasm-pack`, `wasm32-unknown-unknown` target) are **auto-installed** on first run — no manual setup needed.

Work is not complete until it passes cleanly.

## Fast iteration sequence

When iterating locally, use this order:

1. Targeted crate checks first (for faster feedback):
   - `cargo test -p <crate>`
2. Then full gate:
   - `cargo xtask check`

## Code quality checks (beyond the gate)

The gate catches formatting, lint, and test failures. These additional checks should be applied during code review:

### No recursion in core algorithms
- Use iterative loops with an explicit stack or `while let` with a `Vec`.
- Scan for functions that call themselves: any `fn foo(...)` whose body contains `foo(` is a violation.
- Route tree traversal, JSON serialization, fragment graph walking — all must be iterative.

### No unnecessary cloning
- Prefer `std::mem::take` or `std::mem::replace` over `.clone()` when the original is no longer needed.
- Function signatures should take ownership (`value: Value`) when the value will be consumed, and borrow (`value: &Value`) when only reading.
- Move clone decisions to the caller, not inside the function.

### No dead code
- Remove unused functions, imports, and variables rather than suppressing with `#[allow(dead_code)]`.
- Check for functions that were superseded by refactoring but never deleted.
- For TypeScript: verify exports are actually imported by consumers. Unexported internal modules should not appear in the public API.

### Security in route params
- Route parameters extracted from URLs must be sanitized (strip `..` traversal sequences and null bytes).
- Never render route-derived values with raw/unescaped output without explicit developer opt-in.
- Document XSS risks where unescaped rendering is available.

### DRY across language boundaries
- When the same logic exists in both Rust and TypeScript (e.g., route matching), evaluate whether one side can be eliminated.
- Prefer server-as-source-of-truth over duplicating logic in the client.
- When serialization code is repeated across consumers, move it into the library (e.g., `render_partial` returns complete JSON — consumers don't assemble it).

### Public API surface
- Export the minimum necessary. Internal utilities should not be in the public API.
- One function that does the whole job is better than three that must be composed.
- FFI functions should return complete responses, not fragments that consumers must assemble.
- Provide typed responses (TypeScript interfaces, doc-commented structs) — not raw strings.

### Documentation completeness
- Every `pub` function needs a `///` doc comment.
- Every module needs a `//!` module-level doc comment.
- Complex logic blocks (>10 lines) need inline comments explaining **why**, not just what.
- TypeScript public and private methods need `/** */` JSDoc.
- User-facing docs (`docs/`, `README.md`) must be updated in the same change as API changes.

### Modern platform usage (Chromium-only)
- Use `URLPattern` API instead of custom path matchers.
- Use `Navigation` API — add `@types/dom-navigation` for types.
- Use `View Transitions` API — add `@types/dom-view-transitions` for types.
- Prefer native browser APIs over custom JS implementations when Chromium supports them.

## Expectations

- Fix reported issues rather than suppressing them.
- Do not merge or commit with a failing gate.
- Keep fixes minimal and scoped to the task.
