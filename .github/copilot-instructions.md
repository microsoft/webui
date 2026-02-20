# WebUI — Copilot Instructions

You are working on **WebUI**, a high-performance server-side rendering framework written in Rust that operates without JavaScript runtimes. It separates static and dynamic content at build time into a binary protocol that enables fast rendering in any host language.

Read and internalize these instructions at the start of every session. They are non-negotiable.

---

## Context you must load first

Before suggesting or applying **any** change, read these files — they are the ground truth:

1. **`DESIGN.md`** — The living technical specification. Architecture, protocol schema, module contracts, and behavioral rules all live here. Treat every constraint in it as mandatory unless the user explicitly asks to change one.
2. **`Cargo.toml`** (workspace root) — Workspace members, dependency versions, and release profile.
3. **`clippy.toml`** — Lint policy (bans `unwrap`/`expect`, caps cognitive complexity at 20, limits function arguments to 5).
4. **`deny.toml`** — Allowed licenses and advisory ignore-list.
5. The specific crate(s) under `crates/` that are relevant to the task at hand.

---

## The one rule that gates everything

Before creating **any** commit, run:

```bash
cargo xtask check
```

This executes, in order: `fmt → clippy → deny → test → build`. Work is **not done** until this passes cleanly. If it fails, fix every reported issue before proceeding. No exceptions.

---

## Performance is the top priority

Every decision — API design, data structure choice, algorithm, error path — must be evaluated through a performance lens first. WebUI's value proposition is speed; nothing else matters if it is slow.

### Hard constraints

| Rule | Rationale |
|------|-----------|
| **No recursion** in core algorithms | Use iterative loops with an explicit stack. Recursion blows the call stack on deep templates and defeats branch prediction. |
| **No regular expressions** in core logic | Deterministic scanners are faster and more predictable. |
| **Minimal runtime computation** | Move work to build time (the `webui build` CLI step) whenever possible. |
| **Protobuf binary serialization** via `prost` | Zero-copy decoding; JSON is for `webui inspect` debugging only. |
| **Buffer consolidation** | Reuse buffers, pre-allocate, avoid unnecessary allocations. |

### Allocation discipline

- `Vec::with_capacity` / `String::with_capacity` when size is known or estimable.
- `push_str` / `write!` into existing buffers — never `format!` in hot paths.
- No unnecessary `.clone()` — pass `&str`, `&[T]`, or slices. Use `Cow<'_, str>` when a value is sometimes borrowed, sometimes owned.
- Prefer explicit state machines and stack-based traversal over recursive AST walking.

### Measurement

- Identify hot paths first: parsing, expression evaluation, handler rendering, protocol serialization, state lookups.
- Measure **before** changing anything: `cargo bench -p <crate>` in `--release` mode.
- After the change, re-measure and report the delta (qualitatively if exact numbers aren't available).
- The smallest safe change that improves CPU time, allocation count, or cache locality wins. Do not over-engineer.

---

## Rust architecture standards

### Error handling
- Library crates (`webui-parser`, `webui-handler`, `webui-expressions`, `webui-state`, `webui-protocol`, `webui-ffi`) use **custom error enums** via `thiserror`.
- Binary crates (`webui-cli`, `xtask`) may use `anyhow`.
- **No `unwrap()` or `expect()`** in library code — `clippy.toml` enforces this.
- Errors must be **actionable**: tell the caller what went wrong *and* what they can do about it.

### Public API surface
- Expose the minimum necessary. Use `pub(crate)` for internal helpers.
- New public functions, structs, traits, and error variants must be documented with `///` doc-comments.
- Use `#[must_use]` on functions whose return value should not be silently discarded.

### Code organization
- One concern per module. When a file approaches ~400 lines, split it.
- Types are `PascalCase`, functions are `snake_case`, constants are `SCREAMING_SNAKE_CASE`.
- `cargo fmt --all` is the sole formatting authority — never override or disable it.

### Dependencies
- Keep them minimal. Prefer the standard library. Before adding a crate, justify why std doesn't suffice.
- Every dependency must pass `cargo deny check` (license allowlist + security advisories).
- Prefer well-maintained crates under MIT or Apache-2.0.

### Safety
- No `unsafe` without a `// SAFETY:` comment explaining the invariant upheld.
- Prefer zero-copy borrowing over cloning. Prefer slices over owned collections when lifetime allows.
- Keep cognitive complexity under 20 per function (enforced by clippy). Refactor, don't suppress.

### Concurrency (when applicable)
- State explicit `Send + Sync` bounds.
- Prefer `tokio::sync::mpsc` channels over `Arc<Mutex<_>>`.
- Use atomics when a mutex is overkill.

---

## Tests are mandatory

Every code change ships with tests. No exceptions.

| Scenario | Requirement |
|----------|-------------|
| New public API | At least one unit test per function/method. |
| Bug fix | A regression test that **fails** without the fix. |
| Performance change | Benchmark comparison (before/after). |
| Refactor | Existing tests must continue to pass unchanged. |

- Unit tests live alongside code in `#[cfg(test)]` modules.
- Integration tests go in each crate's `tests/` directory.
- Run the **targeted** crate first (`cargo test -p <crate>`), then the full workspace (`cargo test --workspace`).
- Never remove, weaken, or `#[ignore]` an existing test unless the user explicitly asks.

---

## DESIGN.md is the living specification

`DESIGN.md` is not documentation — it **is** the specification. Code implements what `DESIGN.md` describes.

- **Read it** before any architectural or API change.
- **Update it** in the same commit whenever you add, remove, or modify a public API, protocol field, fragment type, error variant, or behavioral contract.
- Keep its Rust code examples conceptually compilable and in sync with real code.
- If `DESIGN.md` and the code disagree, that is a bug — fix both.

---

## Developer docs (`/docs`) stay current

The `docs/` directory is a VitePress site for external developers consuming WebUI.

- Any change to user-visible behavior, CLI usage, or public API **must** include a corresponding docs update in the same PR.
- New features get a guide page (`docs/guide/`) or tutorial (`docs/tutorials/`).
- Verify with `cd docs && pnpm build` when possible.

---

## Branch and commit discipline

- **Never commit to `main` directly.** Create a branch: `<user>/<short-description>` (e.g. `mmansour/optimize-handler-allocs`).
- One logical change per commit. Write imperative messages: *"Add …"* not *"Added …"*.

---

## Commands reference

| What | Command |
|------|---------|
| **Full gate (run before every commit)** | `cargo xtask check` |
| Format | `cargo xtask fmt` |
| Lint | `cargo xtask clippy` |
| License & advisory audit | `cargo xtask deny` |
| Tests (workspace) | `cargo xtask test` |
| Build (workspace) | `cargo xtask build` |
| Test a single crate | `cargo test -p webui-parser` (or any crate name) |
| Benchmark a crate | `cargo bench -p webui-protocol` (or any crate name) |
| Build in release mode | `cargo build --release` |
| Docs site | `cd docs && pnpm build` |

---

## FFI boundary (`webui-ffi`)

The FFI crate exposes WebUI to **any** host language (C, C#, Go, Ruby, Python, Node.js, etc.) via a C-compatible ABI. Treat it as the project's most sensitive surface.

- Every `pub extern "C" fn` must have a `# Safety` doc section explaining pointer validity, lifetime, and ownership expectations.
- All `unsafe` blocks require a `// SAFETY:` comment. No exceptions.
- Assume callers are in a different language with no Rust safety net — validate every input (null pointers, invalid UTF-8, out-of-range values) before dereferencing or converting.
- Never panic across the FFI boundary. Catch all errors and return them as error codes or null pointers. A panic in FFI is undefined behavior.
- C header generation is handled by `cbindgen` in `build.rs`. After changing any `#[no_mangle]` function signature, verify the generated header in `include/webui_ffi.h` is correct.
- Keep the FFI surface minimal and stable — additions are easy, removals break every consumer.
- Platform-specific code must be gated behind `#[cfg(target_os = "...")]` and every platform path must be tested or at least compile-checked.
- Design for ABI stability: prefer opaque pointers and integer error codes over exposing Rust struct layouts.

---

## Protobuf schema evolution

The protocol is defined in `crates/webui-protocol/proto/webui.proto` and compiled by `prost` via `build.rs`. Schema changes cascade through the entire stack: **protocol → handler → FFI → CLI**.

- Never remove or renumber existing proto fields — mark them `reserved` instead.
- Add new fields as optional with sensible defaults so older serialized data remains decodable.
- After any `.proto` change, rebuild the full workspace (`cargo xtask build`) and run all tests — not just the protocol crate.
- Update `DESIGN.md` protocol section in the same commit.

---

## Workspace dependency management

All third-party dependency versions are centralized in the root `Cargo.toml` under `[workspace.dependencies]`.

- **Never** add a dependency version directly in a crate-level `Cargo.toml`. Use `dep = { workspace = true }` instead.
- Before adding any new dependency, check if the standard library or an existing workspace dependency covers the need.
- New dependencies must pass `cargo deny check` (license allowlist + advisory audit).

---

## Release profile awareness

The workspace ships with an aggressive release profile (`Cargo.toml`):

```toml
[profile.release]
lto = true            # Full link-time optimization
codegen-units = 1     # Maximum optimization (slower compile)
panic = "abort"       # No unwinding — smaller binary, but panics terminate immediately
strip = true          # Strip debug symbols
```

- **`panic = "abort"` means panics kill the process instantly** — reinforcing why `unwrap`/`expect` are banned in library code.
- Always validate performance claims in `--release` mode. Debug builds are not representative.
- Be aware that LTO + single codegen unit makes release builds slow. Use `cargo test` (debug) for iteration, `cargo build --release` for final validation.

---

## Shared test utilities (`webui-test-utils`)

The `webui-test-utils` crate provides common test helpers, builders, and fixtures.

- Before writing new test helpers, check if `webui-test-utils` already has what you need.
- New shared test utilities belong in `webui-test-utils`, not duplicated across crates.
- Add it as a `[dev-dependencies]` entry: `webui-test-utils = { path = "../webui-test-utils" }`.

---

## Acceptance checklist

Before finishing any task, confirm **all** of these:

- [ ] `cargo xtask check` passes.
- [ ] Changes include or update tests.
- [ ] `DESIGN.md` is updated if any contract changed.
- [ ] `docs/` is updated if any user-facing behavior changed.
- [ ] No new recursion or regex in core paths.
- [ ] No new `unwrap`/`expect` in library code.
- [ ] No unnecessary allocations introduced; buffers reused where possible.
- [ ] FFI changes include `# Safety` docs and never panic across the boundary.
- [ ] Proto schema changes are backward-compatible and cascade-tested.
- [ ] New dependencies use `workspace = true` and pass `cargo deny check`.
- [ ] Commit is on a feature branch, not `main`.
- [ ] Commit message is imperative with Copilot co-author trailer.
