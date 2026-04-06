---
name: perf
description: Speed and memory performance rules for Rust crates, webui-framework, and webui-router.
---

# Performance

WebUI's value proposition is speed and low memory usage. Every change to core Rust crates, `@microsoft/webui-framework`, or `@microsoft/webui-router` must be evaluated through two lenses: **throughput** (how fast) and **memory** (how little).

Server memory is not cheap. Client memory is not unlimited. Every allocation that can be avoided is a win on both sides.

Use this skill when modifying any performance-sensitive code across the stack.

## Rust - speed rules

These apply to `webui-handler`, `webui-state`, `webui-expressions`, `webui-parser`, `webui-protocol`, and `webui-ffi`.

1. **No `format!()` in writer output.** Use sequential `writer.write()` calls. `format!` allocates a temporary `String` every invocation.
2. **No `.to_string()` on `Cow`.** Write `Cow<str>` directly to avoid defeating zero-copy.
3. **No `collect::<Vec<_>>()` on splits.** Iterate `path.split('.')` directly. Collecting allocates a `Vec` for sequential access.
4. **No redundant scans.** Use `first_part.len() == path.len()` instead of `path.contains('.')`.
5. **No `String::from(ch)` in escape loops.** Use `ch.encode_utf8(&mut buf)` with a `[u8; 4]` stack buffer, or batch contiguous safe chars into a single write.
6. **No `format!` in hex parsing.** Use direct arithmetic (`(hi_nibble << 4) | lo_nibble`) instead of `u8::from_str_radix(&format!(...))`.
7. **No per-request template re-parsing.** Pre-parse at protocol load time and reuse.
8. **No silent `unwrap_or` defaults.** If `binding_stack.pop()` returns `None`, that's a protocol error - propagate it, don't mask it.

## Rust - memory rules

1. **No cloning large state trees.** Use `evaluate_with_resolver` with a closure. Cloning duplicates the entire JSON tree in memory.
2. **No cloning HashMaps for scope.** Save/restore only the overwritten key on loop iteration. A HashMap clone copies every entry.
3. **No cloning `String` values for read-only access.** Use `s.as_str()` for `Value::String` branches; only create owned strings for `Number`/`Bool` via a scratch buffer or `Cow`.
4. **Pre-allocate buffers.** Use `Vec::with_capacity` / `String::with_capacity` when size is known or estimable. For HTML output, 4096 bytes is a reasonable starting point.
5. **Prefer `&str` and slices over owned types.** Pass by reference when the callee only reads. Move clone decisions to the caller.
6. **Use `Cow<'_, str>` when a value is sometimes borrowed, sometimes owned.** Avoids unconditional allocation.
7. **No deep-cloning protocol or state per request.** Use `Arc<T>` with clone-on-write or snapshot swapping.
8. **Cap memory for untrusted inputs.** File reads during discovery must have size limits. A 100MB HTML file should not cause OOM.

## TypeScript - `@microsoft/webui-framework` rules

These apply to `packages/webui-framework` (the client-side Web Component runtime).

### Speed

1. **Single-pass hydration.** The framework walks the DOM once to connect all bindings. No multi-pass scanning.
2. **Path-indexed targeted updates.** When an `@observable` changes, only bindings referencing that property are visited - not the entire template.
3. **DOM cloning over innerHTML.** Use `cloneNode(true)` from cached template fragments. Never use `innerHTML` for component creation.
4. **Delegated events.** One listener per event type on the shadow root, not one closure per element. Reduces listener count by orders of magnitude.
5. **Microtask coalescing.** Multiple property changes within the same synchronous block batch into a single DOM update via `queueMicrotask`.
6. **Cursor-based repeat reconciliation.** `<for>` block updates use a diff algorithm that only calls `insertBefore` on nodes that actually moved. Append/prepend/remove are O(1).

### Memory

1. **No framework in the GC.** Minimize object allocations during reactive updates. Reuse binding objects, don't recreate them.
2. **Template cache is `WeakMap`-keyed.** Parsed template DOMs are cached per metadata object. When metadata is released (e.g., via `Router.releaseTemplates()`), the cache entry becomes GC-eligible.
3. **No per-update array allocations.** Avoid `.filter()`, `.map()`, `.slice()` in the update hot path. Use index-based iteration.
4. **Strip SSR markers after hydration.** Comment nodes used as markers are removed from the DOM once wiring is complete - they don't persist as memory overhead.
5. **Scope frames are stack-allocated.** `<for>` loop item variables use a linked-list scope chain, not cloned Maps or Objects.

## TypeScript - `@microsoft/webui-router` rules

These apply to `packages/webui-router` (the client-side SPA router).

### Speed

1. **Server does route matching.** The client does not re-match routes. The server returns the matched `chain` array; the client diffs old vs new and mounts only changed components.
2. **Lazy loading via dynamic import.** Route component JS is fetched only on first navigation to that route.
3. **Chain diffing, not full remount.** Navigating between sibling routes preserves parent components. Only the changed level is remounted.

### Memory

1. **Release unused templates.** `Router.releaseTemplates()` clears cached component templates for routes the user hasn't visited recently. Active route components are never released.
2. **Inventory bitmask prevents duplicate downloads.** The server tracks which component templates the client already has via a bitmask. Re-navigation never re-sends templates.
3. **Minimal state per navigation.** Route-scoped state means the JSON partial contains only what the active route needs, not the full app state.

## Measuring

### Rust benchmarks

```bash
cargo bench -p microsoft-webui --bench contact_book_bench          # full run
cargo bench -p microsoft-webui --bench contact_book_bench -- --test # quick validation
cargo xtask bench all                                               # all crates
```

Compare **Render/1000 P50** before and after. Verify output **Bytes** is unchanged (same HTML = correct behavior).

### Client performance

```typescript
window.addEventListener('webui:hydration-complete', () => {
  for (const entry of performance.getEntriesByType('measure')) {
    if (entry.name.startsWith('webui:hydrate:')) {
      console.log(`${entry.name}: ${entry.duration.toFixed(1)}ms`);
    }
  }
});
```

### What to report

When making a performance-related change, report:
- **Before/after benchmark numbers** (P50 latency, throughput)
- **Allocation count delta** if measurable
- **Output size** unchanged (proves correctness)
- **Memory profile** for memory-related changes (heap snapshots, RSS delta)
