---
name: ffi
description: Rules and checklist for safe, stable changes to the webui-ffi C ABI boundary.
---

# FFI Boundary Workflow

Use this skill when touching `crates/webui-ffi` or C ABI signatures.

## Mandatory safety rules

- Every `pub extern "C" fn` must include a `# Safety` doc section.
- Every `unsafe` block must include a `// SAFETY:` justification comment.
- Never allow panic to cross the FFI boundary:
  - **No `unwrap()` or `expect()`** in any FFI code path (including error-handling helpers).
  - **Wrap all FFI function bodies** in `std::panic::catch_unwind()` — panics from transitive dependencies (serde, prost, handler) can cross the boundary even if your code is careful.
  - On panic, set the last-error message and return a null/error sentinel.
- Validate all foreign inputs before dereferencing or conversion:
  - null pointers
  - invalid UTF-8
  - out-of-range values

## ABI stability expectations

- Keep exported surface minimal and stable.
- Prefer opaque pointers and integer error codes over exposing Rust layouts.
- Use `#[cfg(target_os = "...")]` for platform-specific code paths.

## Header sync

If any `#[no_mangle]` signature changes, verify generated header output in:

- `crates/webui-ffi/include/webui_ffi.h`
