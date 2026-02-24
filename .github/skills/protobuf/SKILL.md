---
name: protobuf
description: Performance-first protobuf schema workflow for WebUI protocol changes.
---

# Protobuf Evolution Workflow

Use this skill when modifying `crates/webui-protocol/proto/webui.proto`.

## Performance-first schema rules

- Prioritize decode speed, memory layout, and smaller payloads over wire compatibility.
- Breaking field changes are allowed when they improve performance measurably.
- When introducing a breaking schema, update all affected layers in one change:
	- protocol
	- handler
	- FFI
	- CLI
- Remove unused fields and message shapes that add decode overhead.

## Required validation

After schema updates, run:

```bash
cargo xtask build
cargo xtask test
```

Schema changes affect the whole stack: protocol → handler → FFI → CLI. Keep the stack synchronized in the same change.

## Documentation sync

When protocol behavior changes, update `DESIGN.md` protocol sections in the same change.