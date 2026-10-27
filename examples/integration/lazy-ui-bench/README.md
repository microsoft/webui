# Lazy template delivery benchmark

Compare `<if>` and `<if lazy>` using the real compiler, build-time template packs,
the framework's `lazy.js` runtime, and production-minified browser bundles. This
is not a benchmark of `w-hydrate="lazy"` or `w-render="lazy"`.

Build the release native addon first. From the repository root on Windows:

```powershell
cargo build --release -p microsoft-webui-node
pnpm --filter @microsoft/webui --filter @microsoft/webui-framework build
$env:WEBUI_ADDON_PATH = Join-Path (Get-Location) 'target\release\webui_node.dll'
node examples\integration\lazy-ui-bench\triggers.mjs
node examples\integration\lazy-ui-bench\bench.mjs
cargo bench -p microsoft-webui --bench lazy_bench
```

On macOS/Linux point `WEBUI_ADDON_PATH` at the release `libwebui_node.dylib` /
`libwebui_node.so`. Playwright Chromium must be installed; `triggers.mjs` uses
Microsoft Edge when available (`WEBUI_BENCH_CHANNEL` overrides it).

## Prefetch triggers and pay-for-play

`triggers.mjs` compiles one app five times: an ordinary `<if>` (**eager**) and
`<if lazy="demand|intent|idle|load">`. Each app has a grid of cards; each card owns
a native `<dialog>` whose 24-field form, repeat, and nested component live in the
body. **s1** has one dialog; **s30** has 30, and the user opens one. Opening sets
the condition, calls `showModal()` once the body has rendered (lazy pages await
`this.$rendered()`; eager pages, which do not import `lazy.js`, flush updates), and
waits for paint.

The run uses Edge with the Lighthouse mobile profile (150 ms RTT, 1.6 Mbps down,
750 Kbps up, 4x CPU), 20 samples after one warmup in alternating order, and fresh
browser contexts. **Cold** has an empty HTTP cache; **warm** first visits the page
and opens the dialog in another tab, so scripts and packs come from the HTTP cache.
That tab is closed 1.5 seconds before the measured page loads: Chromium tears a
closed tab's compositor down about a second later on the renderer thread that
same-origin pages share, which otherwise adds hundreds of milliseconds to an
open that coincides with it. For intent, the driver hovers for 200 ms before
clicking. Idle and load count as started only after every pack is registered.
Static scripts and packs use Brotli 11 and immutable caching; HTML uses Brotli 5
and `no-store`. Memory runs are unthrottled: open and close all 30 dialogs three
times, wait for idle release, force GC, and read the JS heap. Output:
`target/lazy-triggers.json` with every sample, including each open's input
delay, pack wait, render, `showModal()`, and paint segments.

### Recorded result: 2026-10-27

Windows 10.0.26300, Intel Core Ultra 9 285K, Edge 156.0.4314.50, Node 24.12.0,
release addon. Medians unless noted; open is input-to-paint. Every variant,
including eager, opened 15-30 ms slower than in the 2026-10-25 recording on the
same machine, so compare variants within one run.

Startup JavaScript, Brotli 11: eager 25,347 / 26,005 bytes for s1 / s30; with
`lazy.js` 27,112 / 27,686 (**+1,765 / +1,681 bytes**). Each pack is about 1,155
bytes. Apps without lazy regions do not include `lazy.js`, and the rendering core
contains no lazy code, so they pay nothing for the feature.

| Scenario | Variant | Cold startup bytes | Startup heap | Cold open (p90) | Pack wait | Warm open (p90) |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| s1 | eager | 28,333 | 1,449 KiB | 135 (164) ms | - | 201 (258) ms |
| s1 | demand | 29,228 | 1,444 KiB | 308 (337) ms | 183 ms | 178 (255) ms |
| s1 | intent | 29,228 | 1,446 KiB | 137 (196) ms | 0 | 200 (233) ms |
| s1 | idle | 30,657 | 1,499 KiB | 139 (172) ms | 0 | 198 (253) ms |
| s1 | load | 30,657 | 1,498 KiB | 126 (158) ms | 0 | 188 (237) ms |
| s30 | eager | 35,595 | 2,136 KiB | 145 (185) ms | - | 183 (241) ms |
| s30 | demand | 32,049 | 1,866 KiB | 311 (339) ms | 181 ms | 188 (242) ms |
| s30 | intent | 32,043 | 1,880 KiB | 124 (179) ms | 0 | 176 (219) ms |
| s30 | idle | 75,072 | 2,247 KiB | 148 (171) ms | 0 | 178 (251) ms |
| s30 | load | 75,071 | 2,246 KiB | 145 (172) ms | 0 | 193 (294) ms |

Warm opens are slower than cold ones for every variant, eager included: their
render and `showModal()` segments take 1.2 to 1.5 times as long. The cause is
shared by all variants and is not lazy loading.

Warm startup transfers 9,317 bytes for s30 eager and about 4,090 for every lazy
variant. Releasing regions after use, s30:

| Variant | Startup heap | After cycle 1 | After cycle 3 | Regions loaded after release |
| --- | ---: | ---: | ---: | ---: |
| eager | 2,109 KiB | 2,422 KiB | 2,434 KiB | - |
| demand | 1,846 KiB | 2,241 KiB | 2,276 KiB | 0 of 30 in 5/5 runs |

Startup blocking time (sum over 50 ms, through two seconds after hydration) was
0 ms for every s1 variant. For s30 the cold medians were 41 ms eager, 28 ms
demand, 34 ms intent, 41 ms idle, and 43 ms load, with overlapping ranges.

What this shows:

- **Pay for play.** With 30 dialogs that start closed, lazy regions transfer 10.0%
  fewer startup bytes and use 12.6% less startup heap than eager, and 56% fewer
  bytes on a repeat visit. After three cycles of using all 30, the heap is 6.5%
  below eager because closed regions are released.
- **Prefetch hides the network.** Intent, idle, and load opens wait 0 ms for packs
  and match eager latency. Intent costs nothing at startup, so it is the default.
- **Demand pays for the network.** A cold demand open adds one round trip (150 ms
  RTT here). From the HTTP cache its pack arrives in about 12 ms, within this
  run's variation between variants. Unthrottled, an earlier probe measured a
  cached demand open about 25 ms slower than one triggered without input: the
  response is delivered in a task after the click's frame.
- **Prefetching everything is not free.** Idle or load prefetch of all 30 regions
  transfers and retains more than eager. Use those triggers for the few regions a
  user is likely to open soon.
- **One small region costs more than it saves.** For s1, lazy adds the runtime
  and a request but saves no meaningful startup heap.

For comparison, an earlier run of this workload on the same machine measured the
previous endpoint design at 321 / 310 ms for a cold demand open (s1 / s30), with
4.4 KB more startup JavaScript than eager. Its start-time preload of all 30
regions added 107 ms of startup blocking time; packs now install one per network
task.

## Startup, open, and retention by body size

`bench.mjs` compares one owner with 25, 250, or 1,000 controls in a lazy body:

- **eager-local**: ordinary `<if>`, templates and state already local; no open request.
- **eager-fresh**: ordinary `<if>`, templates local, fresh state fetched on every open.
- **lazy**: `<if lazy="demand">`; opening fetches state and calls
  `this.$prefetch()` in parallel, then awaits `this.$rendered()`.

`WEBUI_BENCH_SAMPLES` defaults to 20 (plus two discarded warmups);
`WEBUI_BENCH_LATENCY_MS` defaults to 40 ms of state-provider delay. Fresh browser
contexts alternate execution order. Heap readings follow forced GC; closed-body
weak-reference results are reported separately. Output:
`target/lazy-browser-proof.json`.

### Recorded result: 2026-10-27

Same machine and addon, Playwright Chromium. Medians.

| Controls | Mode | Startup HTML + JS gzip | Initial heap | Cold open | Warm open | Closed body retained |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 25 | eager-fresh | 21,837 | 1,859,348 | 51.2 ms | 54.7 ms | 20/20 |
| 25 | lazy | 23,784 | 1,857,408 | 51.1 ms | 54.4 ms | 0/20 |
| 250 | eager-fresh | 23,556 | 1,887,480 | 55.1 ms | 48.6 ms | 20/20 |
| 250 | lazy | 23,785 | 1,857,408 | 53.7 ms | 48.7 ms | 0/20 |
| 1,000 | eager-fresh | 29,317 | 1,980,568 | 56.4 ms | 47.4 ms | 20/20 |
| 1,000 | lazy | 23,782 | 1,857,408 | 56.2 ms | 49.0 ms | 0/20 |

Lazy startup no longer grows with the body. The fixed cost at 25 controls is
1,947 gzip bytes, against 4,152 with the earlier endpoint design. Since the
2026-10-25 recording, applications without lazy regions became 247 gzip bytes
smaller because the rendering core no longer carries lazy hooks, and the lazy
runtime grew by 130 gzip bytes. Lazy delivery breaks even just above 250 controls
(229 bytes more at 250), and 1,000 controls save 19% of startup bytes and 123 KB
of heap. Because the pack loads in parallel with state, opening matches
eager-fresh. Eager-local, which requests nothing, opens in 1.2-4.6 ms.

The release Rust benchmark (`lazy_bench`) compares closed and open SSR for 250
controls and measures rendering one pack at build time.

## UI responsiveness

```powershell
node --test examples\integration\lazy-ui-bench\responsiveness.test.mjs
node examples\integration\lazy-ui-bench\responsiveness.mjs
```

Output: `target/lazy-responsiveness.json`, including individual trusted events,
all trials, median/p95, input delay, long tasks, and frame gaps. The defaults are
20 measured contexts per case after two warmups, 250/1,000 controls, native and
4x CPU throttling, and the same 40 ms state-provider delay. The driver sends real
mouse and keyboard input and checks `Event.isTrusted`. It measures cold opening,
a shell click while data is pending, single-field and whole-form typing, closing,
and warm reopening.

Event Timing measures input to the next paint with 8 ms quantization and a 16 ms
reporting threshold; unreported fast events are kept as `<16 ms`. This is a
laboratory measurement, not field INP. Long Tasks and rAF gaps expose blocking
during later mounting.

Earlier recordings showed the same keydown-to-paint latency for eager and lazy
bodies, and long tasks for 1,000-control mounts at 4x CPU in both. Lazy delivery
postpones and releases work; it does not make a mounted control tree faster.
Very large regions still need smaller bodies or progressive rendering.
