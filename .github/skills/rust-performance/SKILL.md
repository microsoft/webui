---
name: rust-performance
description: High-performance Rust workflow for this repository. Use this for optimization, profiling, allocation reduction, throughput/latency improvements, and performance-sensitive refactors.
---

# Purpose

Use this skill when working on performance-critical Rust changes in this repository.

# Required context first

Before suggesting or applying changes, read and use:

1. `DESIGN.md` (authoritative architecture and constraints).
2. `Cargo.toml` at the repository root (workspace and release profile assumptions).
3. The target crate `Cargo.toml` and source files under `crates/` relevant to the task.

Treat these `DESIGN.md` constraints as mandatory unless the user explicitly asks to change them:

- No recursion for core algorithms.
- No regular expressions in core logic.
- Favor minimal runtime computation.
- Minimize allocations (buffer consolidation, reuse, avoid unnecessary clones).
- Preserve strict context isolation and actionable error handling.

# Performance-first workflow

Follow this sequence for Rust performance work:

1. Identify hot path(s): parsing, expression evaluation, handler rendering, protocol/state transformations.
2. Measure before change when possible:
   - Run targeted benches if available (for example protocol benches).
   - Use release mode for realistic performance checks.
3. Propose the smallest safe change that improves one of:
   - CPU time / algorithmic complexity
   - allocations / copies
   - branch predictability / cache locality
4. Validate correctness with targeted tests before broad checks.
5. Re-measure after change and report deltas qualitatively if exact metrics are unavailable.

# Repository-specific commands

Prefer these commands for this repository:

- `cargo test -p webui-protocol`
- `cargo test -p webui-parser`
- `cargo test -p webui-handler`
- `cargo test -p webui-expressions`
- `cargo test -p webui-state`
- `cargo bench -p webui-protocol`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo build --release`

If command cost is high, run the narrowest command that validates the touched crate first, then broaden as needed.

# Rust optimization guidance

- Prefer iterative loops over recursive traversal.
- Prefer explicit state machines/stack structs over recursive AST walking.
- Avoid regex-based parsing; use deterministic scanners/parsers.
- Avoid `unwrap` and panic-driven control flow; return typed errors.
- Reduce temporary allocations:
  - pre-allocate with `with_capacity` when size can be estimated,
  - append into existing buffers/writers,
  - avoid `format!` in tight loops when direct `push_str` or `write!` is possible.
- Avoid unnecessary cloning; pass references and slices where ownership transfer is not required.
- Keep public behavior unchanged unless user asks for semantic change.

# Change acceptance checklist

Before finishing, confirm all items:

- Relevant sections of `DESIGN.md` were consulted and followed.
- No new recursion or regex introduced in performance-critical paths.
- A targeted test/bench/lint command was run for impacted crate(s).
- Any tradeoffs (readability vs speed, memory vs CPU) are stated briefly.

# Invocation hints

To explicitly invoke this skill in Copilot CLI prompts, use `/rust-performance`.
Examples:

- `Use /rust-performance to optimize state path lookup allocations in webui-state.`
- `Use /rust-performance to improve parser throughput without regex or recursion.`