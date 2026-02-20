# Performance

The WebUI protocol and compiler are designed for efficient runtime consumption by platform handlers.

## Protobuf Binary Format

The protocol output uses Protocol Buffers binary serialization instead of JSON:

- **Smaller binary size** — protobuf encoding is significantly more compact than equivalent JSON
- **Faster deserialization** — binary parsing avoids the overhead of JSON string tokenization and UTF-8 validation
- The format is purpose-built for machine consumption; use JSON output for human-readable debugging

## Serialization

- **Pre-allocated buffers** — serialization buffers are sized to `encoded_len()` before writing, avoiding incremental reallocations
- **Buffer consolidation** — fragment data is consolidated into fewer, larger buffers to reduce the total number of allocations

## Algorithm Design

- **No recursion** — all traversal and serialization algorithms are iterative only, using explicit stacks where needed. This avoids stack overflow on deeply nested templates and provides predictable memory usage.