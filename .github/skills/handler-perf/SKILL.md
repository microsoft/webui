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

## Benchmark

```bash
cargo bench -p webui --bench contact_book_bench          # full run
cargo bench -p webui --bench contact_book_bench -- --test # quick validation
cargo xtask bench webui                                   # via xtask
```

Compare **Render/1000 P50** before and after changes. Verify output **Bytes** is unchanged (same HTML = correct).
