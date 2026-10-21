# Lazy template delivery benchmark

Compare `<if>` and `<if lazy>` using the real compiler, production-minified
browser bundles, and identical rendered controls. This is not a benchmark of
`w-hydrate="lazy"` or `w-render="lazy"`.

From the repository root on Windows:

```powershell
cargo build --release -p microsoft-webui-node
pnpm --filter @microsoft/webui build
$env:WEBUI_ADDON_PATH = Join-Path (Get-Location) 'target\release\webui_node.dll'
node examples\integration\lazy-ui-bench\bench.mjs
cargo bench -p microsoft-webui --bench lazy_bench
```

On macOS/Linux point `WEBUI_ADDON_PATH` at the freshly built release
`libwebui_node.dylib` / `libwebui_node.so`. Playwright Chromium must be installed.

The browser run writes `target/lazy-browser-proof.json`, including every sample,
environment, median, and p90. `WEBUI_BENCH_SAMPLES` defaults to 20 (plus two
discarded warmups); `WEBUI_BENCH_LATENCY_MS` defaults to 40 ms of state-provider
delay. Set it to zero to isolate local processing. An optional argument changes
the output JSON path.

Three cases expose the trade-off rather than hiding it:

- **eager-local**: ordinary `<if>`, templates and state already local; no open request.
- **eager-fresh**: ordinary `<if>`, templates local, fresh state fetched on every open.
- **lazy**: `<if lazy>`, metadata and fresh state fetched together; warm inventory
  suppresses metadata on later opens.

All cases use the same core renderer and optional template collection. Only lazy
loads the delivery/renderer adapters. Total compressed startup bytes include those
extra modules, not just the smaller SSR document. Each sample checks every rendered
label/input value and confirms closed bodies have no DOM.

Fresh browser contexts alternate execution order. Heap readings follow forced GC;
closed-body weak-reference results are reported separately. The live ordinary owner
must retain its eager template, whereas an unused lazy body can be collected.
These measurements describe this synthetic workload, not universal latency or
whole-process memory savings. A small body may cost more because of loader code;
cold lazy opening cannot beat an already-local body over a network.

## Recorded result: 2026-10-21

Windows 10.0.26300, Intel Core Ultra 9 285K, Node 24.12.0, Chromium 153.0.8010.12,
esbuild 0.28.2. Release native addon; 20 browser samples per case, alternating
order, two discarded warmups. Values below are medians unless described otherwise.

For 1,000 identical controls, comparing ordinary `<if>` with fresh-state requests
against `<if lazy>`:

| Measurement | Eager-fresh | Lazy | Difference |
| --- | ---: | ---: | ---: |
| Initial HTML, raw bytes | 69,139 | 889 | -98.7% |
| Total startup HTML + JS, gzip bytes | 29,856 | 26,243 | -12.1% |
| Definition + hydration activation | 3.50 ms | 1.80 ms | -48.6% |
| Initial JS heap after GC | 1,983,400 bytes | 1,871,832 bytes | -5.6% |
| JS heap after opening, closing, and collecting | 2,134,140 bytes | 2,051,136 bytes | -3.9% |
| Closed body metadata still retained | 20/20 runs | 0/20 runs | Owner remains connected |
| Cold open, with 40 ms provider delay | 50.50 ms | 53.00 ms | +2.50 ms |
| Warm open, with 40 ms provider delay | 55.40 ms | 51.40 ms | -4.00 ms |

Activation p90 was 3.80/2.10 ms; cold-open p90 was 60.10/62.10 ms. Warm lazy
responses contained 278 raw bytes and no templates, versus 67,726 raw bytes for
the first lazy response. Timing spreads are in the JSON; the warm-open result
is not a universal speed guarantee.

The 0 ms provider-delay repeat showed the same 1,000-control startup bytes and
heap. Cold open was 14.0 ms eager-fresh versus 17.8 ms lazy; warm open was
11.3/11.1 ms. Eager-local, which performs no data request, opened in 3.4 ms with
the default run. Lazy delivery is a startup/memory trade-off, not free I/O.

Small bodies do not necessarily win: total gzip startup was **24,118 bytes eager
versus 26,242 lazy at 250 controls**, and **22,412 versus 26,242 at 25 controls**.
The optional loader cost is included. Prefer ordinary `<if>` for small,
frequently/immediately used UI; lazy helps sufficiently large, initially unused
UI, particularly when the delivery runtime is already needed.

The release Rust benchmark uses 250 controls, 100 Criterion samples, two seconds
of warmup, and five seconds of measurement per case. Plain rendered HTML is
asserted equal for both open and closed states:

| Measurement | Eager | Lazy |
| --- | ---: | ---: |
| Closed SSR bytes | 17,544 | 825 |
| Closed SSR median (95% CI) | 7.025 us (6.980-7.089) | 3.512 us (3.490-3.545) |
| Initially open SSR median (95% CI) | 14.393 us (14.196-14.809) | 15.307 us (15.233-15.459) |

Closed SSR is about 50% faster here, but initially open SSR is about 6.4% slower.
Renaming `defer` to `lazy` itself provides no speedup. No server allocation-count
or whole-renderer/RSS reduction is claimed; the memory figures above are JS heap.

## UI responsiveness

Run the separate interactive fixture with the same release addon:

```powershell
node --test examples\integration\lazy-ui-bench\responsiveness.test.mjs
node examples\integration\lazy-ui-bench\responsiveness.mjs
```

Output: `target/lazy-responsiveness.json`, including individual trusted events,
all trials, median/p95, input delay, long tasks, and frame gaps. The defaults are
20 measured contexts per case after two warmups, 250/1,000 controls, native and
4x CPU throttling, and the same 40 ms state-provider delay.

This fixture is a fixed-column grid of native inputs in a scrollable section,
without virtualization or `content-visibility`. All controls exist after opening.
Its toolbar, CSS, and interactivity are opt-in; the original startup benchmark's
HTML/JS/gzip sizes remain unchanged for all nine cases.

The driver sends real browser mouse clicks and keyboard input, and checks
`Event.isTrusted`. It measures cold opening, another shell click while data is
pending, five single-field edits with a reactive preview, five edits broadcasting
to every value binding, closing, and warm reopening. Output checks prove that
single-field edits leave the other controls untouched and broadcast edits update
every control. There is no forced GC during interaction measurements.

**Measurement boundaries matter:**

- Browser **Event Timing** measures click/keydown to the next paint, with 8 ms
  quantization and a 16 ms reporting threshold. Unreported fast events are
  retained as `<16 ms`, not omitted or replaced with zero. This is a laboratory
  interaction measurement, **not field INP**.
- Opening can paint a loading state before the form exists. Its asynchronous
  **form-readiness** measurement therefore runs separately, from the click
  timestamp through DOM readiness and two animation frames. It is a render-
  opportunity proxy, not an assertion of the exact pixel-presentation time.
- Long Tasks and rAF gaps expose blocking during later mounting. A responsive
  loading indicator does not prove a jank-free form mount.
- 4x CPU is a reproducible simulation on this desktop, not evidence from a
  physical low-end device. Typing uses 100 keystrokes per case across 20 contexts;
  the five keystrokes within each context are not independent page loads.

### Recorded interaction result: 2026-10-21

Same software/hardware as above. Values are **median / p95 milliseconds**.
Eager-fresh and lazy both fetch fresh state on each opening.

| CPU | Controls | Interaction | Eager-fresh next paint | Lazy next paint |
| --- | ---: | --- | ---: | ---: |
| Native | 250 | Single-field typing | <16 / 16 | <16 / 16 |
| Native | 1,000 | Single-field typing | <16 / 16 | <16 / 16 |
| 4x | 250 | Single-field typing | 24 / 24 | 24 / 24 |
| 4x | 1,000 | Single-field typing | 32 / 40 | 32 / 40 |
| Native | 250 | Whole-form typing | 16 / 16 | 16 / 16 |
| Native | 1,000 | Whole-form typing | 16 / 16 | 16 / 24 |
| 4x | 250 | Whole-form typing | 32 / 48 | 32 / 56 |
| 4x | 1,000 | Whole-form typing | 96 / 152 | 104 / 168 |

Both fresh-state modes painted the open-button feedback at p95 16 ms natively
and 24 ms at 4x CPU for both counts. The shell click actually occurred while
loading in 20/20 cases for each fresh-state mode/count/CPU combination; it had
no observed long task. This measures input while awaiting data, not input
deliberately injected into the later mount task.

Cold **form-ready two-rAF proxy**, including the 40 ms provider delay:

| CPU | Controls | Eager-fresh | Lazy |
| --- | ---: | ---: | ---: |
| Native | 250 | 76.4 / 86.5 ms | 71.8 / 84.6 ms |
| Native | 1,000 | 77.4 / 88.5 ms | 80.4 / 91.9 ms |
| 4x | 250 | 115.3 / 136.7 ms | 127.4 / 152.3 ms |
| 4x | 1,000 | 225.8 / 370.1 ms | 250.7 / 346.9 ms |

At 4x CPU, both modes encountered long tasks in **all 20 cold opens** of 1,000
controls: maximum 262 ms eager-fresh and 233 ms lazy, with maximum rAF gaps
300.1/283.3 ms. Whole-form typing encountered long tasks in **97/100 keystrokes**
in both modes. At native speed neither fresh-state mode observed a long task in
these measured actions. The JSON also records warm opening and closing.

**Conclusion:** lazy delivery postpones work and reduces startup/retention costs;
it does not make the mounted control tree intrinsically faster. A 1,000-control
mount or global update can still visibly stall a CPU-constrained browser.
Single-field edits are much cheaper than broadcast updates. These measurements
do not establish a steady-state responsiveness improvement from `<if lazy>`.
