---
name: code-review
description: Framework code review checklist - correctness, performance, concurrency, design, and style.
---

# Framework Code Review

Use this skill when reviewing or writing framework-level code: handler, parser, protocol, router, CLI runtime, FFI, or state management. The checklist is generic - apply every section to every PR that touches core logic.

> **Complements**: `perf` (speed and memory optimization), `quality-gate` (automated checks), `ffi` (C ABI boundary safety).

---

## 1 - Cross-layer contract parity

Every behavioral contract that spans more than one layer (parser → protocol → handler → client) must be **identical** in semantics. Mismatches cause silent divergence between SSR and client rendering.

| Check | Why |
|-------|-----|
| **Encoding / decoding policy** | If one layer decodes (e.g., `decodeURIComponent`), all layers that consume the value must agree on whether the value is decoded or raw. |
| **Matching semantics** | Optional, splat, exact, and wildcard rules must produce the same result on server and client for the same input. |
| **Tie-breaking determinism** | When multiple candidates match with equal rank, the selection rule must be documented and identical across layers. |
| **Edge values** | Empty strings, `/`, encoded slashes (`%2F`), Unicode, and values containing delimiters (`.`, `:`, `*`) must be tested at every boundary. |
| **Error representation** | When one layer returns an error, the other layers must handle or propagate it consistently - no silent swallowing. |
| **Spec-vs-code parity** | If `DESIGN.md` specifies behavior (e.g., array indexing `items.0.name`, string `.length` semantics), the implementation must match. Tests that lock in spec-violating behavior are bugs, not proof of correctness. |
| **Binding surface parity** | Node, WASM, and CLI bindings must expose the same logical API. If one uses protobuf bytes and another uses JSON strings, transparent fallback is impossible. |
| **Type coercion consistency** | If `==` uses strict equality but `>` coerces strings to numbers, document the coercion matrix. Operators on the same pair of operands should not produce contradictory outcomes. |
| **Literal grammar completeness** | If the expression engine supports `true`, `false`, `"strings"`, and numbers as literals, it must also support `null`. Missing literals create dead-end conditions (e.g., `foo == null` fails). |
| **Fallback contract validity** | If a function falls back to a CLI subprocess or alternative backend, the result shape must satisfy the same postconditions as the primary path. An empty buffer from a fallback is a contract violation. |

### How to check

- Write a table: "Input X → Layer A produces Y₁, Layer B produces Y₂". If Y₁ ≠ Y₂, file a parity bug.
- Ensure tests exist that exercise the same input through all layers.
- Diff the function signatures between Node/WASM/TS bindings - arguments and return types must match logically.
- Search for tests that assert spec-violating behavior (e.g., `test_array_index_not_resolved`) - these are bugs masquerading as tests.

---

## 2 - Determinism

Non-deterministic behavior is a correctness bug, even if it "usually works."

| Anti-pattern | Fix |
|-------------|-----|
| Iterating `HashMap` / `HashSet` for order-dependent logic (ranking, tie-breaking, first-match) | Use `BTreeMap`, `IndexMap`, or add an explicit ordering field. |
| Relying on allocation or hash seed order across process restarts | Use deterministic comparison (insertion order, lexicographic, priority field). |
| Floating-point comparison for equality | Use integer arithmetic or epsilon-aware comparison. |
| `rand` / `Uuid::new_v4` in deterministic paths | Inject randomness explicitly; keep core logic pure. |

### How to check

- Search for `HashMap` or `HashSet` usage where iteration order affects output.
- Run the same input twice in different process instances - output must be identical.

---

## 3 - Concurrency and race safety

### Rust (server / CLI)

| Check | Why |
|-------|-----|
| **Lock scope** | Hold `Mutex` / `RwLock` only long enough to copy or read what you need. Never hold a lock across I/O, rendering, or network calls. |
| **Snapshot consistency** | If a request reads multiple fields from shared state, read them all under **one** lock acquisition. Two separate locks can produce mixed old/new data if a writer updates between them. |
| **`std::sync::Mutex` on async paths** | Blocking mutexes on async paths can starve the executor. Prefer `tokio::sync::RwLock` or `Arc<ArcSwap<T>>` for read-heavy shared state. |
| **Atomics** | Use `AtomicU64` for simple counters/versions instead of locking. |
| **File watcher error recovery** | If a rebuild fails, the watcher must NOT update its "last seen" timestamps - otherwise it won't retry when the error is transient. Keep a retry/backoff path. |
| **Cache concurrent writes** | Temp files used for atomic-replace must have unique names (PID + random suffix). Two writers using the same `{key}.tmp` will clobber each other. |
| **Event-driven vs polling** | Polling every N ms scales poorly. Prefer event-driven file notifications (`notify` crate) or at least directory-level mtime checks before deep scans. |

### Rust (FFI / WASM boundaries)

| Check | Why |
|-------|-----|
| **`catch_unwind` on FFI exports** | Every `extern "C"` function must wrap its body in `std::panic::catch_unwind()`. Panics from transitive dependencies (serde, prost) crossing the FFI boundary are UB in unwind mode and abrupt termination in abort mode. |
| **WASM panic hooks** | Install `console_error_panic_hook` (or equivalent) so panics produce diagnosable errors instead of opaque host traps. |
| **Callback error propagation** | If a host callback (e.g., JS chunk writer) fails, stop processing immediately. Continuing to render after the sink has failed wastes CPU and can produce partial/corrupt output. |

### TypeScript (client)

| Check | Why |
|-------|-----|
| **In-flight request cancellation** | Rapid user actions (navigations, searches) can launch multiple async operations. Use `AbortController` + sequence IDs to discard stale results. |
| **Event ordering** | DOM events and `CustomEvent` dispatches can interleave with async completions. Ensure state mutations are guarded by a generation counter or similar. |
| **Feature detection** | Browser APIs (`Navigation API`, `View Transitions`) may not exist. Guard with `typeof` / `'navigation' in window` before use. |

### Node.js bindings

| Check | Why |
|-------|-----|
| **Event loop blocking** | Synchronous native calls that do parsing, rendering, or I/O block the Node.js event loop. Expose async variants via `napi::Task`/`AsyncTask` for any operation that takes > 1ms. |

---

## 4 - Allocation and clone discipline

Every unnecessary allocation is CPU time and GC pressure. Apply these rules in hot paths (request handling, rendering, matching, serialization).

### Rust

| Anti-pattern | Fix |
|-------------|-----|
| `format!()` in loops or hot paths | Use `write!(buf, ...)` into a pre-allocated buffer, or `push_str`. |
| `String::from(ch)` for single characters | Use `buf.push(ch)` or `write!(buf, "{ch}")`. |
| `.clone()` on a value used only for read access | Pass `&T` or `&str` instead. |
| `.clone()` to insert into `HashSet` after a `.contains()` check | Reorder: do the lookup / get first, then `insert(owned_value)` by move. |
| `.to_string()` on `Cow<str>` | Write the `Cow` directly; `.to_string()` defeats zero-copy. |
| `collect::<Vec<_>>()` when sequential iteration suffices | Iterate the iterator directly. |
| `Vec<T>` when max size is small and known | Use `SmallVec<[T; N]>` or a stack array to avoid heap allocation. |
| Deep-cloning large structures (protocol, state trees) per request | Use `Arc<T>` with clone-on-write or snapshot swapping. |
| Re-parsing static data on every request | Cache the parsed form alongside the raw data. |
| `to_vec()` before `join()` | Slice the original and join directly: `segments[idx..].join("/")`. |
| Recompiling parser queries per call | Cache compiled queries in `OnceLock<Query>` or extract all attributes once into a struct per element. |
| `chars[i..].iter().collect::<String>()` for substring comparison | Compare without allocation: use byte-level `starts_with` or slice equality. |
| Zero-capacity `String::new()` for output buffers | Pre-allocate with `String::with_capacity(estimated_size)`. For HTML output, 4096 is a reasonable starting point. |
| Cloning JSON `Value` subtrees on lookup | Add a borrowed lookup API (`-> &Value`) for the hot path; keep owned API as a thin wrapper. |
| Full-tree validation pre-pass on every evaluation | Validate once at AST construction time and cache the result, not on every evaluation call. |

### TypeScript

| Anti-pattern | Fix |
|-------------|-----|
| `Array.filter().length` for counting | Use a `for` loop with a counter, or `reduce`. |
| Repeated DOM queries (`querySelectorAll`) on unchanged DOM | Cache the result; invalidate on mutation. |
| `JSON.parse(JSON.stringify(x))` for deep copy | Use `structuredClone()` or avoid the copy entirely. |
| Rebuilding derived data every call when inputs haven't changed | Memoize or cache with invalidation. |

---

## 5 - Data structure selection

Choose the structure that matches the access pattern. A wrong choice is a design bug.

| Access pattern | Preferred structure |
|---------------|-------------------|
| Order-dependent iteration (ranking, priority) | `BTreeMap`, `Vec` sorted by key, `IndexMap` |
| Membership test only | `HashSet`, bloom filter (but beware false positives - see below) |
| Frequent insert + lookup by key | `HashMap` (if order doesn't matter) |
| Small fixed-size collections (< 16 elements) | Stack array, `SmallVec`, `ArrayVec` |
| Append-only log | `Vec` with `push` |
| Probabilistic membership (bloom filter) | Use ≥ 2 independent hash functions. A single-hash bloom filter has unacceptable false-positive rates at moderate load. Document the false-positive bound. |

### How to check

- For every `HashMap` in the diff: "Does any consumer iterate this and depend on order?" If yes, switch.
- For every `Vec`: "Is the capacity known?" If yes, use `with_capacity`.
- For every bloom filter: "What is the false-positive rate at expected load?" If > 1%, redesign.

---

## 6 - API surface and design deduplication

| Check | Why |
|-------|-----|
| **No duplicate logic across layers** | If server and client both implement matching, both must call equivalent code, not independent reimplementations. Any behavioral change must update both. |
| **No duplicate calls in the same request** | If `match_route()` is called twice on the same path in one request flow, extract the result and pass it through. |
| **Free functions vs. methods** | If standalone helper functions duplicate methods on a type (e.g., `activateRoute(el)` and `el.activate()`), remove one. Keep the method; delete the free function. |
| **Caching static computations** | Parsing a template string into segments is pure. If the template set is static, parse once and cache. |
| **Minimal public surface** | Use `pub(crate)` for internal helpers. Every public API is a maintenance commitment. |
| **Error paths are actionable** | Errors must say what went wrong AND what the caller can do. "Parse error" is not actionable. "Invalid parameter name 'id-x' in path '/users/:id-x': only alphanumeric and underscore allowed" is. |

---

## 7 - Type safety, correctness idioms, and architectural rules

### Rust - banned patterns

| Pattern | Preference |
|---------|-----------|
| `unwrap()` / `expect()` in library code | **Banned.** Use `?` with typed errors via `thiserror`. |
| `unsafe` without `// SAFETY:` comment | **Banned.** Document the invariant upheld. |
| Recursion in core algorithms | **Banned.** Use iterative loops with an explicit stack. Recursion blows the call stack on deep/adversarial inputs and defeats branch prediction. |
| `as any` casts (in FFI or interop) | Minimize. Use typed wrappers or trait objects. |
| Raw pointer arithmetic | Prefer safe abstractions. Use `unsafe` only when necessary, with safety proof. |

### Rust - required annotations

| Pattern | Preference |
|---------|-----------|
| `#[must_use]` | Add to all public constructors, serialization/deserialization methods, and functions returning `Option`/`Result` whose value should not be silently discarded. |
| `#[allow(dead_code)]` | Remove the annotation AND the dead code. If the field/function is planned for future use, delete it now and re-add when needed (with a TODO referencing an issue). |

### Rust - error handling precision

| Pattern | Preference |
|---------|-----------|
| Silent fallbacks (`unwrap_or(0)`, `unwrap_or_default()`) | Replace with explicit error propagation. A silent default can mask a real mismatch (e.g., unbalanced start/end markers). If a default is genuinely correct, add a comment explaining why. |
| `Option<T>` return for lookups that can fail in multiple ways | Use `Result<T, E>` with a typed error enum distinguishing missing-key, invalid-path, type-mismatch, etc. Collapsing all failures to `None` makes debugging impossible. |
| Error variants that abuse a generic `Validation(String)` | Add dedicated variants (`Decode`, `Encode`, `MissingField`) so callers can match programmatically. |
| Byte vs char index confusion | When scanning strings with `chars()` but slicing with byte indices, panics occur on non-ASCII input. Always use `char_indices()` or work exclusively with byte offsets. |

### Rust - numeric precision

| Pattern | Preference |
|---------|-----------|
| Forcing all numbers through `f64` for comparisons | Preserve exact integer comparisons (`i64`/`u64`) when both operands are integral. Only fall back to `f64` for genuine floats. Large integers above 2^53 lose precision in `f64`. |

### Rust - build-time validation

| Pattern | Preference |
|---------|-----------|
| Deferring validation to render-time | Validate at parse/build time and fail fast. If a route references a component that doesn't exist, or a path is empty, catch it during parsing - not when a user hits the route. |
| Accepting invalid inputs that pass through | If a `<route>` element is missing `path` or references an unknown component, the parser must reject it with an actionable error. |
| `bytes[i] as char` on UTF-8 strings | This silently corrupts multi-byte characters. Use `str` slicing or `char_indices()` to preserve UTF-8 correctness. |

### TypeScript

| Pattern | Preference |
|---------|-----------|
| `(x as any)` | Use proper type narrowing, generics, or `unknown` with guards. |
| Untyped expando properties (`(el as any)._data = ...`) | Use `WeakMap<Element, T>` or typed fields on a subclass. |
| `var` | **Banned.** Use `const` by default, `let` only when reassignment is needed. |
| `== null` | Use `=== null \|\| === undefined` or optional chaining (`?.`). |
| Callback-based async | Use `async/await`. |
| `arguments` object | Use rest parameters (`...args`). |
| Loose union types (`state: object \| string`) | Use precise types. If Rust accepts `serde_json::Value`, define a recursive `JsonValue` type on the TS side. If plugin accepts `"fast"` only, use a string literal union. |

---

## 8 - Style, comments, and documentation

| Check | Rule |
|-------|------|
| **Dead code** | Remove unused functions, imports, and variables. Don't comment them out. |
| **Commented-out code** | Delete it. Git has history. |
| **Obvious comments** | Remove comments that restate the code: `// increment counter` above `counter += 1`. |
| **Missing comments** | Add comments for non-obvious decisions: "We use BTreeMap here for deterministic iteration order." |
| **`TODO` / `FIXME` / `HACK`** | Acceptable only with an associated issue number: `// TODO(#123): switch to streaming parser`. |
| **Naming** | Rust: `snake_case` functions, `PascalCase` types, `SCREAMING_SNAKE` constants. TS: `camelCase` variables/functions, `PascalCase` classes/types. |
| **Magic numbers** | Extract to named constants with a doc comment. |
| **File length** | When a file exceeds ~400 lines, split by concern. |
| **Function arguments** | Max 5 parameters. Use a config/options struct beyond that. |
| **Unused dependencies** | Remove deps listed in `Cargo.toml` that are never imported. `anyhow` is for binary crates only - library crates use `thiserror`. |
| **Placeholder / no-op APIs** | If a public function always returns empty/default, either implement it or remove it behind a feature gate. Silent no-ops mislead callers. |
| **Tests that lock in wrong behavior** | A test named `test_feature_not_supported` that asserts `None` for spec-required behavior is a bug, not a test. Delete or fix it. |

---

## 9 - Server runtime, proxy, and discovery correctness

When the change touches dev-server, API proxy, request handling, or package discovery:

| Check | Why |
|-------|-----|
| **HTTP semantics** | `Accept` header parsing must respect quality weights (`q=`). Substring matching (`contains("application/json")`) is incorrect per RFC 7231. |
| **Status codes** | A "no match" must return 404, not 200 with empty body. Clients cannot distinguish "valid empty" from "not found" otherwise. |
| **Proxy header forwarding** | Forward a safe allowlist: `Authorization`, `Cookie`, `Accept`, `Content-Type`, `X-Request-ID`, etc. Don't forward `Host` or hop-by-hop headers. |
| **Path handling** | Paths with dots (e.g., `/api/v2.1/users`) are valid routes, not file requests. Don't use `contains('.')` as a file-vs-route heuristic. |
| **Resource reuse** | HTTP clients, TLS contexts, and connection pools are expensive to create. Reuse them across requests. |
| **HMR injection** | Search for closing tags case-insensitively. `</BODY>` and `</body>` are both valid HTML. |
| **Input validation** | CLI commands must validate that input paths are directories (not files) before proceeding. Actionable error early beats cryptic failure later. |
| **Cache invalidation scope** | Cache keys must include hashes/mtimes of ALL files that affect the cached result - not just `package.json`. If template HTML or CSS changes, the cache must invalidate. |
| **Cache write safety** | Use per-write unique temp file names (PID + random suffix) and atomic rename. Concurrent writers must not clobber each other's temp files. |
| **Symlink policy** | Package resolvers must allow workspace-linked packages (symlinks outside `node_modules`). Blocking all external symlinks breaks common monorepo patterns. |
| **File-size guardrails** | Discovery that reads arbitrary files must cap read sizes. A 100MB HTML file shouldn't cause OOM during component scanning. |

---

## 10 - Protocol and schema hygiene

When the change touches `.proto` files, serialization, or the binary protocol:

| Check | Why |
|-------|-----|
| **Field additions are backward-compatible** | New fields must have defaults that preserve old behavior. |
| **Removed fields use `reserved`** | Prevents accidental reuse of field numbers. |
| **Enum variants are exhaustive** | Match arms must handle unknown variants (use a default/unknown variant or `_` with a log). |
| **No unnecessary nesting** | Flat structures decode faster than deeply nested ones. |
| **Measure payload size** | Before/after byte counts for representative inputs. Extra fields add decode overhead even when empty. |
| **Prioritize decode speed** | Breaking field changes are allowed when they improve performance measurably. Remove unused fields and message shapes that add decode overhead. |
| **Cascade all layers** | Schema changes affect the whole stack: protocol -> handler -> FFI -> CLI. Update all in the same change. Run `cargo xtask build && cargo xtask test` to validate. |
| **Update DESIGN.md** | Protocol behavior changes must update the protocol sections of `DESIGN.md` in the same commit. |

---

## 10b - FFI boundary safety

When the change touches `crates/webui-ffi` or C ABI signatures:

| Check | Why |
|-------|-----|
| **`# Safety` doc on every `extern "C" fn`** | Documents the invariants the caller must uphold. |
| **`// SAFETY:` on every `unsafe` block** | Explains why the unsafe operation is sound. |
| **`catch_unwind` wraps all FFI bodies** | Panics from transitive deps (serde, prost) crossing the boundary are UB in unwind mode. On panic, set last-error and return null. |
| **Validate all foreign inputs** | Null pointers, invalid UTF-8, out-of-range values - check before dereferencing. |
| **No `unwrap()` or `expect()`** | Anywhere in the FFI code path, including error-handling helpers. |
| **Minimal stable surface** | Prefer opaque pointers and integer error codes over exposing Rust layouts. |
| **Header sync** | If any `#[no_mangle]` signature changes, verify `crates/webui-ffi/include/webui_ffi.h`. |

---

## 11 - Documentation sync

Every behavioral change must include corresponding documentation updates. Missing docs are a review finding.

| Check | Why |
|-------|-----|
| **DESIGN.md updated** | If the change modifies public APIs, protocol fields, behavioral contracts, error variants, or SSR markers, `DESIGN.md` must be updated in the same commit. |
| **User-facing docs updated** | If the change affects CLI flags, template syntax, component authoring, routing, or integration behavior, `docs/` must be updated. |
| **AI reference updated** | If the change affects anything a code-generation AI would need to know, `docs/guide/ai.md` must be updated. |
| **README links to docs portal** | Package READMEs should defer to the docs portal, not duplicate content. |
| **No stale examples** | Code examples in docs must use current API signatures, flag names, and marker formats. |
| **docs build passes** | `cd docs && pnpm build` must succeed (catches broken links, unescaped `{{`, missing pages). |

See `docs-sync` skill for the full file mapping of what-changed to which-docs-to-update.

---

## Review output format

When producing a code review, report findings as:

```
### ISSUE-NNN: <short title>

**Severity**: 🔴 Bug | 🟠 Security | 🟡 Performance | 🔵 Design/Style
**Files**: `path/to/file.rs` (lines N-M)
**Description**: What is wrong, with a code snippet showing the problem.
**Impact**: What breaks or degrades, and under what conditions.
**Fix**: Concrete suggestion, with a code snippet when helpful.
```

Each finding must be **self-contained** - readable without context from other findings. This allows direct conversion to GitHub issues.
