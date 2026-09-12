# Actual `StreamingWriter` transport experiments

This extends `streaming_resource_bench`, not the external `ChannelWriter`
benchmark. No production writer/default changes are made here. A bounded
4/16/64 KiB writer is an experiment, not an endorsed configuration.

## Build and untimed validation

From the workspace root:

```sh
cargo build --locked --release -p microsoft-webui --example streaming_resource_bench
cargo test --locked --release -p microsoft-webui --example streaming_resource_bench
target/release/examples/streaming_resource_bench --transport smoke hard
target/release/examples/streaming_resource_bench \
  --transport http-smoke hard 16384 pooled progressive 8388608 1 1
```

Use `soft` for a binary linked against the original writer. `smoke` checks all
seven workloads, all three sizes, pooled/unpooled, and one byte-paced drain.
It does not collect latency samples. `http-smoke` verifies five complete HTTP
bodies and stops/joins its local server. Always pass `--transport`: the example's
legacy no-argument behavior still runs its old resource measurements.

For the historical comparison, use identical umbrella
`e274c8b80cb54ecc562ab4528f4ca4d93dd1d9a4` trees plus this benchmark patch.
Restore **only** `crates/webui/src/streaming.rs` from
`f43db65067d48581ed7f99acc4b8d68f201c8bb9` in the soft tree. This holds dependency
lockfile, renderer, state, fixtures, compiler flags and harness constant.
The bounded tree keeps the umbrella writer/pool unchanged. Thus the primary
comparison includes the explicit writer **and pool** differences; unpooled rows
separate out pooling. A pool-only comparator may be added by restoring just
that independently reviewed streaming source into a third otherwise identical
tree. `pool-only` is a source-contract label, not an algorithm implementation.

Create isolated detached trees with `git worktree add --detach`, never switch
an existing checkout. Apply the same benchmark patch to each. Build with
distinct absolute `CARGO_TARGET_DIR` values (`target-soft`, `target-hard`,
optionally `target-pool-only`), and preserve `git diff`, `rustc -Vv`, lockfile,
source hashes and binary hashes. Do not copy archived files over a shared Cargo
target: equal mtimes can make a stale build look current. There are no new
dependencies or public APIs.

## Commands and explicit timing approval

Positional syntax:

```text
--transport MODE VARIANT CHUNK_BYTES POOL_MODE WORKLOAD RATE SAMPLES ITERATIONS [POOL_BYTES]
```

- Modes: `account`, `measure`, `http-smoke`, `http-measure`.
- Variants: `soft`, `hard`, `pool-only`; labels do not replace the linked writer.
- Chunk sizes: 4096, 16384, 65536; the default 4096 construction path is retained
  exactly (no gratuitous `with_chunk_size` call).
- Pool modes: `pooled`, `unpooled`. The shared pool always has 16 slots.
- Pool bytes default to configured chunk bytes. An optional final value permits
  a **matched** legacy-headroom sensitivity check, e.g. 5120 in both soft and
  hard-4096 runs, or a matched pool-only comparison. The original unpooled writer
  allocates target + 1024 bytes; its pooled replacement path may also grow
  buffers. These are intentional source differences, not hidden adjustments.
- Rate: bytes/second, 0 for unrestricted concurrent draining.

Example untimed accounting and a later explicitly approved timed run:

```sh
target/release/examples/streaming_resource_bench \
  --transport account hard 65536 pooled raw_1m 8388608 1 1
WEBUI_TRANSPORT_MEASURE=1 target/release/examples/streaming_resource_bench \
  --transport measure hard 65536 pooled raw_1m 8388608 9 1
```

Timing modes refuse to run without `WEBUI_TRANSPORT_MEASURE=1`. Do not set it
until competing builds/tests have stopped and measurements are approved.

## Workloads

| Name | Constant work/output shape |
|---|---|
| `tiny` | 32 eight-byte writes; 256 bytes total |
| `small_16k` | 1024 sixteen-byte writes; 16 KiB total |
| `mixed` | Full HTML shell plus 512 product rows, interleaved raw text, quoted attributes and boolean attributes; UTF-8 and entity text |
| `raw_1m` | One exactly 1 MiB UTF-8 raw value |
| `attribute_1m` | The same 1 MiB value via the specialized `write_attribute` method, plus its fixed framing |
| `ssr_1000` | Existing real contact-book build/state fixture with 1000 contacts, rendered at `/contacts` (not the small dashboard); the full list uses the real handler |
| `progressive` | Real parsed two-boundary document with a 512-item loop, state serialization, shell/checkpoint/terminal flushes |

Input preparation/parsing and reference construction are outside measurement.
The reference uses the handler's generic attribute formatting. Untimed passes
assert every received byte, all explicit flush positions, final length, and
hard maximum (when labelled hard). SHA-256 and flush offsets are emitted for
cross-binary comparisons. UTF-8 may split inside a hard chunk: only the complete
byte stream is compared/decoded. Unit tests also reject checkpoint coalescing.

## Methodology and metric interpretation

The transport harness keeps one consumer OS thread, one four-slot Tokio channel,
and one shared pool alive per cell. One response at a time is written by the
calling thread; the other drains concurrently and acknowledges final release.
Thread/channel/pool creation is excluded from request timing. Writer construction,
channel backpressure, chunk release, and control/ack scheduling are included.
No active Tokio runtime and no flush timeout are used in transport-only rows.

Pacing is based on cumulative bytes from **first body arrival**, including the
last chunk. After consuming N bytes the consumer cannot complete before
`first_arrival + N/rate`; it holds one chunk while servicing that budget.
This is not a fixed delay per chunk, and a giant soft chunk does not bypass its
byte budget. Scheduler overshoot and actual sleep/wakeup counts still differ.
Tiny chunks can incur more wakeups; those are real harness scheduling costs,
not evidence about a physical network by themselves.

`account` performs cold-pool and warm-pool passes, with five warmups before the
latter. Output verification, chunk/max accounting and allocation counters are
**untimed**. Allocation counts use the existing global System wrapper: alloc and
alloc_zeroed plus growing realloc calls; requested bytes count realloc growth
deltas, not total traffic through realloc. They include both harness threads,
not just `StreamingWriter`. Pool/channel objects and reference setup are excluded.
The legacy example's allocator counters are disabled during timing, removing
per-allocation atomic increments; a relaxed enabled-flag read remains. Thus
these are same-instrumentation comparisons, not instrumentation-free baselines.

Observed idle buffer **counts** are separate from source-derived capacity bounds:

| Hard chunk size | Four queued payloads | 16-slot idle pool capacity bound |
|---|---:|---:|
| 4 KiB | 16 KiB | 64 KiB |
| 16 KiB | 64 KiB | 256 KiB |
| 64 KiB | 256 KiB | 1 MiB |

Larger chunks intentionally increase queue-byte and pool-byte budgets. These
bounds exclude active producer buffers, a pending send, consumer-held chunks,
spare capacity, ownership/channel metadata, state and serialization scratch.
Original soft writes have no input-independent queue-payload maximum; the
original pool also has no configured idle-capacity maximum. JSON `null` means
there is no source-derived bound, not zero memory.

`process_rss_high_water_bytes` is a process-lifetime peak, including setup and
allocator arenas, **not current/live memory, retained pool bytes, or a per-render
delta**. Use separate processes for cells. This harness does not observe private
Vec capacities or instantaneous queued payload bytes; do not infer them from RSS.

Timed transport JSON retains individual first-body, producer-complete, drained,
and acknowledged-complete request durations, plus batch wall and process
user/system CPU deltas. CPU includes both threads. Divide batch CPU by iterations
for a **mean**, not a median. Preserve raw requests; summarize actual request
medians and distributions separately across process repeats. Do not interpret
Criterion's printed estimates as medians; this matrix does not use them.

The HTTP mode uses real actix-web/awc, one server worker, `spawn_blocking`, the
actual specialized writer methods, and a four-slot body channel.
`TCP_NODELAY` is explicitly enabled and checked on every accepted server socket:
the OS-default Nagle/delayed-ACK interaction can otherwise dominate these small
loopback responses by approximately 40 ms. This socket setting is identical
across writer variants and does not change production host defaults. It reports
header arrival separately from the first **nonempty body chunk**; the unchanged
legacy HTTP example measured headers as TTFB. Five warmup bodies are verified.
HTTP/TCP/kernel and framework buffering can drain transport chunks ahead of a
slow client; source queue bounds do not bound all HTTP buffering. HTTP received
chunk sizes are not writer chunk sizes. There is no browser/FCP/LCP claim.

Recommended primary matrix: soft-default versus hard 4/16/64 KiB, both pool
modes; all workloads unpaced, the five nontrivial workloads at 8 MiB/s, and
mixed/progressive at 1 MiB/s. Use three process rounds with rotated variant
order, 15 × 100 requests for small fast cells, 15 × 20 for large/SSR cells,
and 9 × 1 for paced cells. HTTP confirmation can stay focused on
mixed/progressive/raw-1MiB at unpaced/8-MiB/s with pooling, 9 × 1 requests.
Run the same-size soft-16/64 and matched 5120-byte pool sensitivity checks when
needed to distinguish a hard cap from a larger coalescing target or pool policy.
