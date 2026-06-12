---
name: diagnostics
description: Error handling and build-time diagnostics conventions - Result-not-panic, structured Diagnostics with stable codes, actionable help, color/JSON presentation layering, exit codes, and cold-path performance.
---

# Error Handling & Diagnostics

Use this skill whenever you add, change, or review an error path: a build-time
authoring error, a parser/handler failure, a CLI validation error, or anything
surfaced to a host (FFI/WASM/Node) or a tool/agent. WebUI errors must be
**recoverable**, **actionable**, and **machine-consumable** — for humans and AI
agents alike.

## 1 - Never panic on recoverable input

`panic = "abort"` in the release profile means a panic **kills the process
instantly** — including any FFI/WASM/Node host embedding the framework. Bad
template input, bad CLI input, and bad state are *recoverable* and must return
`Result`, never `panic!`/`unwrap()`/`expect()`.

| Situation | Do |
|-----------|----|
| Malformed template / CSS / route authored by a developer | Return `Result` with a structured `Diagnostic` (see §3). |
| Missing/invalid CLI input (file, port, flag) | Return a typed `CliError` (see §6). |
| A genuinely impossible internal state | Prefer `?` with a typed error; only use `unreachable!`/`expect` with a justification comment, and never in a hot or host-reachable path. |

`unwrap()`/`expect()` are banned in library code (`clippy.toml`
`disallowed-methods`). `todo!`, `unimplemented!`, and `dbg!` are banned
workspace-wide (`clippy.toml` `disallowed-macros`). Tests opt out with
`#[allow(clippy::disallowed_methods)]`.

> **Enforcement note.** `unwrap`/`expect`/`todo!`/`unimplemented!`/`dbg!` are
> caught by clippy. `panic!` is *not* lint-banned (too entrenched) — keep it out
> of recoverable paths by review. The "no regex in core logic" rule is also
> review-enforced, not `deny`-banned: `actix-web` pulls `regex` transitively, so
> a crate-level ban would break the build and can't scope to first-party code.

## 2 - Error type conventions

| Crate kind | Error type |
|-----------|-----------|
| Library (`webui-parser`, `webui-handler`, `webui-expressions`, `webui-state`, `webui-protocol`, `webui-ffi`) | Custom `enum` via `thiserror`. |
| Binary (`webui-cli`, `xtask`) | `anyhow` for orchestration; a typed `enum` when callers must branch on the cause (e.g. `webui-cli`'s `CliError` for hints + exit codes). |

- Each error layer's `Display` describes **only its own level**; the `#[source]`
  chain carries the rest, so `anyhow`'s `{:#}` never double-prints. Provide a
  flat `chain_message()` helper for hosts that don't walk the chain (Node, FFI).
- Add **dedicated variants** instead of overloading a generic `Generic(String)` /
  `Validation(String)` so callers can `match` programmatically.

## 3 - Authoring errors are structured `Diagnostic`s

Every "the developer wrote invalid template syntax" mistake is returned as
`ParserError::Template(Box<Diagnostic>)` (`crates/webui-parser/src/diagnostic.rs`),
so all build errors render identically. A `Diagnostic` carries:

- **`code`** — a stable, machine-readable identifier (e.g. `invalid-for-each`).
  Defined in `diagnostic::codes`. Treat codes as a **stable API**: tools and AI
  agents branch on them, so rename only with a deliberate migration.
- **title** — short, lowercase (`invalid <for> each expression`).
- **location** — set from a byte offset via `.at_offset(source, offset)`;
  rendered rustc-style `--> owner:line:column` (single forward scan, **no regex,
  no recursion**), falling back to `in component <c> · element <e>`.
- **snippet** — the offending source text.
- **`help:`** — an actionable fix (see §4).

Add a new authoring error with the parser helpers (`authoring_error`,
`authoring_error_at`, `html_error`) and a new constant in `diagnostic::codes`.
Validate at **parse/build time and fail fast** — never defer to render time.

## 4 - Make errors actionable (and typo-aware)

Tell the developer **what** is wrong *and* **how** to fix it. Every `Diagnostic`
should carry a `help:` line. Where a mistake is likely a typo, suggest the
intended name via `suggest::closest_match` (iterative Levenshtein — **no
recursion, no regex**, cold path only):

- Misspelled directive attribute: `<for eahc=…>` -> "did you mean `each`?"
- Unknown component tag: suggest the closest **same-namespace** registered
  component (`<mp-buton>` -> `<mp-button>`). **Prefix-guard** the match (text
  before the first `-` must match) so a genuine third-party custom element
  (`<md-button>`) is never falsely flagged.

## 5 - Presentation layering: color belongs ONLY in the entry point

Libraries produce **plain, color-free data**. The entry point decides how to
present it. Never embed ANSI in library output or in any machine/host channel.

| Consumer | Gets |
|----------|------|
| `webui-cli` (terminal) | Reads `Diagnostic` fields and colorizes with `console::style()` — the **only** approved styling method (see copilot-instructions "Terminal output styling"). |
| FFI / WASM / Node | The plain `Display` text through their native error channel (`webui_last_error`, `JsValue`, `napi::Error`). |
| Browser / tools (dev-server live-reload, SSE, `console.error`) | **Plain** text. ANSI renders as garbage and breaks single-line SSE frames. |

When one value feeds both a terminal and a non-terminal channel, split it:
`webui-dev-server`'s `RebuildError { display, message }` carries a colorized
`display` for the reporter and a plain `message` for the browser.

Per-line color: when colorizing multi-line output, style **each line
independently** (open + close the SGR span within the line). A single span that
straddles newlines bleeds when the line is later re-prefixed (e.g. `[server]`
under `xtask dev`).

## 6 - Machine-readable output and exit codes (`webui-cli`)

For editors, CI, and AI/agent tooling:

- `--format json` (global flag) emits each error as one JSON object on
  **stdout** (no ANSI; decorative output suppressed):
  `{severity, code, message, file, line, column, snippet, help, chain}`.
  Branch on the stable `code`, not the human `message`. Build the object with
  the `serde_json::Map` API, **not** the `json!` macro (it `unwrap`s internally
  and trips `disallowed_methods`).
- Exit codes follow BSD `sysexits.h` (`webui-cli`'s `error::exit_code`):
  `65` data/authoring error, `66` missing input, `69` port in use, `74` I/O,
  `2` usage (clap), `1` otherwise.
- Replace fragile `err_msg.contains("...")` dispatch with typed errors that own
  their `hint()` and `exit_code()`.

## 7 - Error construction is COLD - keep it off the hot path

Building a `Diagnostic` (format strings, suggestions, location scans) is rare,
but if it inlines into a hot function it bloats that function and perturbs its
code layout — a real, measurable regression (observed ~4-5% on parse benches
with **no** added hot-path work).

- Mark error builders `#[cold]` + `#[inline(never)]` (e.g. `authoring_error*`,
  `html_error`, `css_diagnostic`, `*_error` constructors, `suggest::closest_match`).
- Keep hot fast-paths inlinable: a per-element check (e.g. `split_once('-')`)
  must stay inlined; only its cold fallback (the registry scan) goes out-of-line.
- Validate layout-sensitive changes with `cargo bench -p <crate>` against the
  base branch (see `skills/perf/SKILL.md`). A "regression" with no added compute
  is usually layout — fix it with `#[cold]`, don't shrug it off.

## 8 - Checklist for a new error

- [ ] Returned as `Result`, never panicked (host-safe under `panic = "abort"`).
- [ ] Authoring mistake -> `ParserError::Template(Box<Diagnostic>)` with a new
      `diagnostic::codes` constant, location, snippet, and `help:`.
- [ ] `help:` is actionable; add a "did you mean …?" suggestion if it's a typo.
- [ ] No color/ANSI in library, host, browser, or JSON output.
- [ ] Surfaced in `--format json` with the stable `code`; exit code classified.
- [ ] Error construction is `#[cold]`/`#[inline(never)]`; hot path unchanged
      (confirm with a benchmark if it sits near a hot loop).
- [ ] Tests: a regression test that fails without the error, asserting on the
      `code` (not the prose); JSON stays plain (no `\x1b`).
- [ ] `DESIGN.md` and `docs/` (incl. `docs/ai.md`) updated if the contract or
      a user-visible code/flag changed.
