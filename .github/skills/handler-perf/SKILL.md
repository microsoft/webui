---
name: handler-perf
description: Performance patterns for the WebUI handler rendering hot path.
---

# Handler Performance

Use this skill when modifying `webui-handler`, `webui-state`, or `webui-expressions`.

## Rules

1. **No cloning large state trees.** Use `evaluate_with_resolver` with a closure instead of cloning state for condition evaluation.
2. **No cloning HashMaps for scope.** Save/restore only the overwritten key on loop iteration.
3. **No `format!()` in writer output.** Use sequential `writer.write()` calls.
4. **No `.to_string()` on `Cow`.** Write `Cow<str>` from `html_escape::encode_safe` directly.
5. **No `collect::<Vec<_>>()` on splits.** Iterate `path.split('.')` directly.
6. **No redundant scans.** Use `first_part.len() == path.len()` instead of `path.contains('.')`.
7. **No `String::from(ch)` in escape loops.** Use `ch.encode_utf8(&mut buf)` with a `[u8; 4]` stack buffer, or batch contiguous safe chars into a single write.
8. **No cloning `String` values for read-only escaping.** Use `s.as_str()` for `Value::String` branches; only create owned strings for `Number`/`Bool` via a scratch buffer or `Cow`.
9. **No per-request route template re-parsing.** Pre-parse templates into `Vec<SegmentPattern>` at protocol load time and store alongside `RouteRecord`.
10. **No `format!` in hex parsing.** Use direct arithmetic (`(hi_nibble << 4) | lo_nibble`) instead of `u8::from_str_radix(&format!(...))`.
11. **No silent `unwrap_or` defaults.** If `binding_stack.pop()` returns `None`, that's a protocol error — propagate it, don't default to `0`.

## Benchmark

```bash
cargo bench -p webui --bench contact_book_bench          # full run
cargo bench -p webui --bench contact_book_bench -- --test # quick validation
cargo xtask bench webui                                   # via xtask
```

Compare **Render/1000 P50** before and after changes. Verify output **Bytes** is unchanged (same HTML = correct).
