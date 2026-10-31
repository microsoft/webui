# Lazy template delivery benchmark

Compare `<if>`, `<if lazy>`, and component assets using the real compiler,
build-time template packs, the generated `lazy.webui.js` module with the
framework's lazy runtime, and production-minified browser bundles. This is not a
benchmark of `w-hydrate="lazy"` or `w-render="lazy"`.

Build the release native addon first. From the repository root on Windows:

```powershell
cargo build --release -p microsoft-webui-node
pnpm --filter @microsoft/webui --filter @microsoft/webui-framework build
$env:WEBUI_ADDON_PATH = Join-Path (Get-Location) 'target\release\webui_node.dll'
node examples\integration\lazy-ui-bench\ntp.mjs
node examples\integration\lazy-ui-bench\triggers.mjs
node examples\integration\lazy-ui-bench\bench.mjs
cargo bench -p microsoft-webui --bench lazy_bench
```

On macOS/Linux point `WEBUI_ADDON_PATH` at the release `libwebui_node.dylib` /
`libwebui_node.so`. Playwright Chromium must be installed; `ntp.mjs` and
`triggers.mjs` use Microsoft Edge when available (`WEBUI_BENCH_CHANNEL`
overrides it). With their defaults, `ntp.mjs` takes about 75 minutes and
`triggers.mjs` about 26. Run one benchmark at a time on an otherwise idle machine.

## NTP-shaped comparison

`ntp.mjs` builds a page shaped like a new tab page in three ways, to compare
`<if lazy>` with component assets, which is how MSN loads on-demand UI today.
Results come from two runs: the full run of 2026-10-29 at `f4e24c25`, and a
bounded rerun of 2026-10-30 at `d6970af6`, which changed how intent and initial
focus are detected. Rows the rerun did not repeat are marked as historical.

The page has a background image as its LCP element (a 223,653-byte JPEG that sits
below a 56 px bar, because Chromium does not count an image that fills the
viewport as LCP), an autofocused search box whose owner holds autosuggest, a
top-sites grid, and ten on-demand UIs:

- Four scripted dialogs, each with 2.3 to 3.3 KB of Brotli-compressed component
  JavaScript: the settings flyout, the market and language dialog, the top-sites
  editor, and the background gallery. Settings and the gallery share a component
  that nothing else uses.
- Six template-heavy menus that are bodies of their owners: the app launcher, the
  sign-in flyout, autosuggest, the top-site context menu, the top-sites toast, and
  the background menu.

Variants:

- **eager**: everything in the startup bundle, with plain `<if>` and static imports.
- **assets** (MSN today): the four dialogs are component assets
  (`build({ componentAssetRoots })`). Their owners start `import()` when the
  pointer enters or focus reaches a dialog's button, or on click, and create the
  dialog with `asset.create()`. The menus are plain `<if>` in their owners.
- **lazy**: all ten UIs are `<if lazy>` regions, and the app imports the generated
  `lazy.webui.js`. Owners with one UI behind one control (settings, market,
  launcher, sign-in, and the search box) use the default intent trigger. The tile
  grid and the background own several UIs each, so their regions use
  `lazy="demand"`, and each button calls `$prefetch('<condition>')` from
  `@pointerenter` and `@focus`.

Assets and lazy are built with `--css link` and `--css style`, eager with link only.

### Method

Each fixture is built the way an application with its own bundler would build it:
Node `build()` writes the generated modules, esbuild bundles the client with code
splitting and the projection plugin, and a second `build()` with the projection
manifest produces the protocol, CSS, and packs. esbuild leaves a split chunk's
shared dependencies as static imports that the browser finds only after the chunk
arrives. webpack and rspack fetch an async chunk group in parallel, so the
harness rewrites each dynamic import to do the same.

A local HTTP/1.1 server (Chromium opens at most six connections to it) serves
scripts, CSS, and packs with Brotli 11 and immutable caching, and HTML with
Brotli 5 and `no-store`. Edge runs at 1280x800 with the Lighthouse mobile profile
(150 ms RTT, 1.6 Mbps down, 750 Kbps up, 4x CPU).

Each measured page load records startup, then opens the settings dialog and the
top-site context menu, in alternating order, in one of three modes:

- **hover**: move the pointer to the button, wait 200 ms, and click.
- **click**: move the pointer and click at once.
- **prefetch**: once the page is ready, prefetch from code (`$prefetch()` for lazy,
  the owner's asset loader for assets), wait until the network is quiet, then click.

An open runs from `pointerdown` to the first frame in which every sentinel
element of the UI is connected, visible, laid out, and styled by its own
stylesheet. Startup lasts until two seconds after hydration and window load.
Startup blocking time (TBT) is the Lighthouse-style sum of long-task time beyond
50 ms between FCP and that point. Startup bytes are transferred bytes, including
the image. JS heap is read after two forced garbage collections.

Each configuration ran 20 rounds after one warmup round, in alternating order,
with fresh browser contexts. That gives 60 startup samples and 20 opens per mode
for each configuration and cache. **Cold** starts with an empty HTTP cache;
**warm** first loads the page and opens both UIs in another tab. Memory runs are
unthrottled: open and close all ten UIs three times and read the heap after each
cycle, five runs per configuration.

Two probes check speculative loading, three runs per build:

- The priority probe holds the window `load` event for 5 seconds, hovers every
  button and calls `$prefetch()` for the toast before `load`, then hovers one more
  button after `load`. It records each request's initial priority from CDP
  `Network.requestWillBeSent`. A request counts as before `load` when its Resource
  Timing start precedes `loadEventStart`; CDP reports `load` only after its
  listeners have run.
- The prerender probe forces `document.prerendering` to true with an init script,
  waits for eager and idle triggers to come due, hovers every button and
  prefetches every UI, then dispatches `prerenderingchange`.

Probe builds change the launcher to `lazy="eager"` and sign-in to `lazy="idle"`
so those triggers are covered too.

Comparisons give the difference of medians with a 95% percentile-bootstrap
interval (4,000 resamples, fixed seed). Within noise means the interval contains
zero. Click-only targets allow one round trip, the transfer time, and two frames
over component assets (two round trips for link builds of UIs with stylesheets).

### Post-fix rerun: 2026-10-30

Source `d6970af6`. Intent is now also heard on owner hosts, so moves within one
shadow tree prefetch, and focus that is already in an owner when the runtime
starts prefetches at low priority only after `load` and idle. Production
`lazy.js` is 2,555 bytes Brotli, and lazy builds' startup JavaScript grew 26 bytes
to 32,579; other sizes are unchanged. Windows 10.0.26300, Intel Core Ultra 9
285K, Edge 156.0.4314.58, Node 24.12.0, esbuild 0.28.2, and the 2026-10-29
release addon (the change is TypeScript only). 168 page loads in 20 minutes,
plus memory and probe runs, with no failures.

The rerun is bounded: assets and lazy in link and style builds, cold and warm,
hover mode only, 20 rounds after one warmup round. That gives 20 startup samples
per configuration and cache instead of 60, so its intervals are wider than the
full run's. Both targets were opened in alternating order, so 10 of each 20
settings hovers came from the tile menu, the case that missed intent before.
Memory (five runs) and both probes (three runs per build) were rerun. Eager,
click-only, and code-prefetch opens were not; their rows come from the full run.

Edge updated itself between runs: 156.0.4314.50 for the full run, .56 for the
2026-10-29 diagnostics, and .58 for this rerun. The component-asset builds, whose
bytes did not change, measured close to the full run (cold assets-link FCP 460
against 452 ms, LCP 1,888 against 1,888, settings hover 146 against 141), so the
two runs can be compared row by row.

Startup (times in ms as median with p90 in parentheses; bytes, requests, and heap
are medians):

| Config | Cache | FCP | LCP | TBT | Bytes | Requests | JS heap |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| assets-link | cold | 460 (476) | 1,888 (1,916) | 0 (0) | 269,595 | 12 | 1,708 KiB |
| lazy-link | cold | 452 (472) | 1,908 (2,052) | 0 (0) | 269,135 | 13 | 1,755 KiB |
| assets-style | cold | 348 (416) | 1,704 (1,728) | 0 (0) | 266,464 | 4 | 1,719 KiB |
| lazy-style | cold | 344 (416) | 1,704 (1,740) | 0 (1) | 266,009 | 5 | 1,764 KiB |
| assets-link | warm | 384 (468) | 384 (468) | 0 (0) | 8,233 | 12 | 1,708 KiB |
| lazy-link | warm | 360 (440) | 360 (440) | 0 (7) | 5,145 | 13 | 1,756 KiB |
| assets-style | warm | 352 (380) | 352 (380) | 0 (0) | 11,167 | 4 | 1,719 KiB |
| lazy-style | warm | 356 (448) | 356 (448) | 0 (5) | 8,072 | 5 | 1,764 KiB |

Lazy's startup blocking time is now 0 ms at the median, cold and warm, against
9 to 15 ms in the full run. The autosuggest pack still loads inside the measured
startup window, now after `load` at low priority, and its region is installed
when the startup heap is read, so the heap difference is unchanged. Three
lazy-link loads and one assets-link load painted the LCP image 185 to 245 ms after
`load` instead of about 50 ms. Those loads set lazy-link's p90, and they are kept.

Hover then click (median ms, p90 in parentheses):

| UI | Cache | assets-link | lazy-link | assets-style | lazy-style |
| --- | --- | ---: | ---: | ---: | ---: |
| settings | cold | 146 (170) | 137 (174) | 128 (169) | 125 (169) |
| context | cold | 34 (54) | 33 (51) | 35 (52) | 34 (55) |
| settings | warm | 140 (175) | 123 (179) | 123 (175) | 117 (171) |
| context | warm | 35 (47) | 33 (53) | 32 (54) | 35 (53) |

Settings, hover then click, cold, by where the pointer came from:

| Pointer path | assets-link | lazy-link | assets-style | lazy-style |
| --- | ---: | ---: | ---: | ---: |
| First UI the pointer reached | 146 (171) | 146 (183) | 138 (174) | 133 (217) |
| From the tile menu | 131 (153) | 123 (161) | 122 (158) | 117 (136) |
| From the tile menu, 2026-10-29 run | 134 (165) | 329 (368) | 145 (181) | 334 (342) |

During the 200 ms hover, lazy-link fetched the same five files as a click-only
open (7,619 bytes) and lazy-style the same three (6,901 bytes); after the click,
lazy-link waited 3 ms for the network at the median.

Memory, median KiB of five runs that agree within 4 KiB:

| Config | Startup | After cycle 1 | After cycle 3 |
| --- | ---: | ---: | ---: |
| assets-link | 1,686 | 2,226 | 2,354 |
| lazy-link | 1,732 | 2,311 | 2,412 |
| assets-style | 1,697 | 2,190 | 2,309 |
| lazy-style | 1,742 | 2,287 | 2,363 |

No lazy region stayed loaded after release in any run.

Speculative loading:

- Before `load`, page loads made no speculative request, and the priority probe's
  hovers fetched only packs, all at low priority: 9 per run in lazy builds and 8
  in probe builds, up from 6 and 7 because hovers inside `ntp-app` now count.
  Their scripts and stylesheets started at `load`. Component assets again
  requested 12 scripts at high priority and, in link builds, 12 stylesheets at
  very high priority before `load`.
- While prerendering, lazy and probe builds made no speculative requests. After
  activation they made 20 in link builds and 15 in style builds.

Acceptance against the original targets, lazy against component assets. Rows
marked 2026-10-29 were not rerun and come from `f4e24c25` on Edge .50.

| Target | link | style |
| --- | --- | --- |
| FCP within noise, cold / warm | Pass: -6 [-18, 4] / -20 [-44, 0] ms | Pass: -4 [-16, 8] / +2 [-20, 10] ms |
| LCP within noise, cold | Pass by the interval: +20 [-4, 36] ms. The full run measured +18 [4, 22] ms on three times the samples, a fail | Pass: +2 [-16, 24] ms |
| LCP within noise, warm | Pass: -20 [-44, 0] ms | Pass: +2 [-20, 10] ms |
| Startup TBT within noise, cold / warm | Pass: +0 [0, 0] / +0 [0, 1] ms | Pass: +0 [0, 0] / +0 [0, 2] ms |
| FCP, LCP, and TBT against eager, cold | 2026-10-29: Pass -4 [-14, 0], Pass -172 [-192, -166], Fail +9 [8, 10] ms | Not measured |
| Startup bytes at or below assets, cold / warm | Pass: 460 / 3,088 B fewer | Pass: 455 / 3,095 B fewer |
| Startup JS heap at or below assets, cold / warm | Fail: +47 [47, 47] / +48 [46, 48] KiB | Fail: +46 [46, 46] / +46 [46, 47] KiB |
| Hover open of settings within noise, cold / warm | Pass: -5 [-30, 23] / -14 [-35, 18] ms | Pass: -5 [-38, 15] / -10 [-31, 16] ms |
| Hover open of the context menu within noise, cold / warm | Pass: -1 [-9, 3] / +0 [-6, 8] ms | Pass: -1 [-6, 7] / +3 [-8, 7] ms |
| Code-prefetch opens within noise | 2026-10-29: Pass | 2026-10-29: Pass |
| Click-only opens within one round trip, in one network round | 2026-10-29: Pass; context menu +178 ms of 188 allowed | 2026-10-29: Pass; context menu +174 of 188 |
| Heap after three cycles below assets | Fail: 2,412 against 2,354 KiB (+2%) | Fail: 2,363 against 2,309 KiB (+2%) |
| Heap after three cycles at least 10% below assets | Fail | Fail |
| No speculative requests while prerendering | Pass: 0 | Pass: 0 |
| No high-priority speculative request before `load` | Pass: 0 of 27, all from the probe | Pass: 0 of 27, all from the probe |

### Full run: 2026-10-29 (historical)

Source `f4e24c25`, before the intent and focus changes. The rerun above replaces
its startup, hover, memory, and priority rows; its eager, click-only, and
code-prefetch rows were not rerun.

Windows 10.0.26300, Intel Core Ultra 9 285K, Edge 156.0.4314.50, Node 24.12.0,
esbuild 0.28.2, release addon at `f4e24c25`. 630 page loads in 73 minutes, with
no failures. Times are medians in ms, with p90 in parentheses.

Sizes, in Brotli bytes:

| Config | HTML | Startup JS (files) | Startup CSS (files) | All packs |
| --- | ---: | ---: | ---: | ---: |
| eager-link | 11,315 | 36,908 (1) | 4,085 (8) | - |
| assets-link | 7,997 | 30,917 (2) | 4,085 (8) | - |
| lazy-link | 4,909 | 32,553 (2) | 4,085 (8) | 10,641 |
| assets-style | 10,930 | 30,905 (2) | - | - |
| lazy-style | 7,836 | 32,553 (2) | - | 14,482 |

At startup, eager-link also preloads the dialogs' five stylesheets (3,933 bytes).
A dialog's deferred bytes differ by at most 230 bytes between assets and lazy
(settings: 6,279 against 6,364 in link builds, 6,205 against 6,139 in style
builds), because lazy moves each dialog's template from its script into its
pack. Lazy also moves the six menus out of the startup page into packs of 396 to
1,666 bytes. The component that settings and the gallery share is repeated in
both packs: 202 bytes in link builds and 404 in style builds, where its CSS
travels in the pack.

Startup, cold:

| Config | FCP | LCP | TBT | Hydrated | Bytes | Requests | JS heap |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| eager-link | 456 (480) | 2,080 (2,112) | 0 (8) | 1,151 | 283,814 | 16 | 1,845 KiB |
| assets-link | 452 (480) | 1,888 (1,920) | 0 (1) | 842 | 269,595 | 12 | 1,708 KiB |
| lazy-link | 452 (472) | 1,908 (1,928) | 9 (20) | 870 | 269,109 | 13 | 1,755 KiB |
| assets-style | 344 (412) | 1,704 (1,732) | 0 (3) | 655 | 266,464 | 4 | 1,719 KiB |
| lazy-style | 340 (396) | 1,716 (1,740) | 10 (16) | 681 | 265,983 | 5 | 1,765 KiB |

Lazy's extra request is the autosuggest pack. At `f4e24c25`, focus already in the
search box when the runtime started counted as intent at once, so the pack loaded
at low priority before `load`.

Warm visits transfer little more than the HTML: eager 11,552 bytes, assets 8,233
(link) and 11,167 (style), lazy 5,145 (link) and 8,072 (style). FCP and LCP
coincide at 356 to 400 ms. Startup TBT is 14 ms for lazy, 0 for assets, and 6 for
eager.

Opens, cold:

| UI | Mode | eager-link | assets-link | lazy-link | assets-style | lazy-style |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| settings | hover | 123 (172) | 141 (173) | 187 (353) | 145 (187) | 164 (340) |
| settings | click | 120 (169) | 325 (358) | 331 (360) | 327 (365) | 325 (343) |
| settings | prefetch | 140 (182) | 131 (171) | 132 (165) | 141 (163) | 127 (146) |
| context | hover | 38 (52) | 33 (54) | 35 (56) | 35 (52) | 35 (50) |
| context | click | 32 (48) | 34 (48) | 212 (220) | 32 (50) | 206 (215) |
| context | prefetch | 33 (48) | 28 (49) | 34 (49) | 32 (48) | 29 (48) |

Click-only opens, cold: requests, bytes, sequential network rounds, and time from
the click to the last response. Eager requests nothing.

| UI | assets-link | lazy-link | assets-style | lazy-style |
| --- | --- | --- | --- | --- |
| settings | 4, 7,280 B, 1, 200 ms | 5, 7,619 B, 1, 220 ms | 2, 6,713 B, 1, 206 ms | 3, 6,901 B, 1, 208 ms |
| context | none | 1, 935 B, 1, 172 ms | none | 1, 935 B, 1, 173 ms |

Lazy-link's five requests for settings are the pack, two script chunks, and two
stylesheets, all in one network round.

The lazy hover result for settings has two modes. Split by where the pointer was
before:

| Settings, hover then click, cold | assets-link | lazy-link | assets-style | lazy-style |
| --- | ---: | ---: | ---: | ---: |
| First UI the pointer reached | 149 (173) | 146 (171) | 142 (187) | 133 (149) |
| Pointer came from the tile menu | 134 (165) | 329 (368) | 145 (181) | 334 (342) |

At `f4e24c25`, the runtime detected intent only with `pointerover` and `focusin`
listeners on the document. Both buttons sit inside `ntp-app`'s shadow tree, so
when the pointer moved from one to the other, the event's target and related
target both retargeted to `ntp-app`, and the browser did not dispatch it past
that shadow root. This is specified DOM behavior; a page without WebUI
reproduces it in Edge 156 and Chromium 153. The document never saw the hover, and
the open took as long as a click-only open. The asset loader and the tile grid's
`$prefetch()` listen on the button itself, so neither was affected. `d6970af6`
also listens on owner hosts; the rerun above shows the result.

Warm hover and prefetch opens were within noise of component assets. Warm
click-only opens of settings were within 9 ms; the context menu took 49 and 53 ms
against 31 and 35, because lazy reads its pack from the HTTP cache.

Memory, median KiB of five runs that agree within 4 KiB:

| Config | Startup | After cycle 1 | After cycle 3 |
| --- | ---: | ---: | ---: |
| eager-link | 1,821 | 2,147 | 2,262 |
| assets-link | 1,687 | 2,227 | 2,355 |
| lazy-link | 1,733 | 2,305 | 2,406 |
| assets-style | 1,697 | 2,190 | 2,310 |
| lazy-style | 1,742 | 2,284 | 2,360 |

No lazy region stayed loaded after release in any run.

Speculative loading:

- Before `load`, lazy builds requested only packs, all at low priority: the
  autosuggest pack in each page load, and 18 packs in the priority probe (21 in
  probe builds). Scripts and stylesheets for the hovered regions started at
  `load`. In the same probe, component assets requested 12 scripts at high
  priority and, in link builds, 12 stylesheets at very high priority before
  `load`. In 6 to 8 of 60 warm loads, the autosuggest pack was requested only
  after `load`, and so at high priority.
- While prerendering, lazy and probe builds made no speculative requests. After
  activation they made 20 in link builds and 15 in style builds.

Acceptance as recorded at `f4e24c25`, lazy against component assets, cold unless
noted:

| Target | link | style |
| --- | --- | --- |
| FCP within noise | Pass: -2 [-12, 2] ms | Pass: -4 [-16, 8] ms |
| LCP within noise | Fail: +18 [4, 22] ms | Fail: +12 [4, 22] ms |
| Startup TBT within noise | Fail: +9 [8, 10] ms | Fail: +10 [8, 11] ms |
| Startup TBT within noise, warm | Fail: +14 [8, 23] ms | Fail: +15 [12, 21] ms |
| Startup bytes at or below assets | Pass: 486 B fewer | Pass: 481 B fewer |
| Startup bytes at or below assets, warm | Pass: 3,088 B fewer | Pass: 3,095 B fewer |
| Startup JS heap at or below assets | Fail: +48 KiB | Fail: +46 KiB |
| Hover open of settings within noise | Fail: +89 [7, 184] ms | Pass, but only because the interval is wide: +84 [-16, 191] ms |
| Hover open of the context menu within noise | Pass: +2 [-11, 12] ms | Pass: +0 [-13, 9] ms |
| Code-prefetch open of settings within noise | Pass: +0 [-22, 9] ms | Pass: -16 [-37, 11] ms |
| Code-prefetch open of the context menu within noise | Pass: +5 [-8, 13] ms | Pass: -4 [-10, 11] ms |
| Click-only open of settings within one round trip | Pass: 331 against 325 ms | Pass: 325 against 327 ms |
| Click-only open of the context menu within one round trip | Pass: 212 against 34 ms, +178 of 188 allowed | Pass: 206 against 32 ms, +174 of 188 allowed |
| Click-only opens fetch everything in one network round | Pass | Pass |
| Heap after three cycles below assets | Fail: 2,406 against 2,355 KiB (+2%) | Fail: 2,360 against 2,310 KiB (+2%) |
| Heap after three cycles at least 10% below assets | Fail | Fail |
| No speculative requests while prerendering | Pass: 0 | Pass: 0 |
| No high-priority speculative request before `load` (cold loads and probe) | Pass: 0 of 78 | Pass: 0 of 78 |

All warm hover and code-prefetch opens passed, and warm click-only opens were
within the allowance. Against eager, which has neither lazy regions nor
component assets, lazy-link's FCP is -4 [-14, 0] ms, its LCP is -172 [-192, -166]
ms, and its startup TBT is +9 [8, 10] ms.

A separate run of 15 cold loads per build of the style variants, on Edge
156.0.4314.56 (installed by Edge's updater after the full run) and the same
`f4e24c25` source, split up the startup costs. Component assets: 1,717 KiB,
LCP 1,692 ms, TBT 0, with no long task after FCP. Lazy: 1,764 KiB, LCP 1,708 ms,
TBT 11, and the task that finished hydration took 54 to 69 ms. Lazy built without
`autofocus` on the search box: 1,730 KiB, LCP 1,700 ms, TBT 0, and that task took
at most 51 ms.

### What this shows

This combines both runs. Startup, hover, memory, and priority statements come
from the 2026-10-30 rerun of `d6970af6`; eager, click-only, and code-prefetch
statements come from the 2026-10-29 run of `f4e24c25`.

For the four scripted dialogs, the new design performs like component assets.
Deferred bytes match within 230 bytes, and opens after a hover, a code prefetch,
or a click take the same time. Both designs fetch everything a dialog needs in
one network round.

Where the new design does better:

- **Repeat visits.** Lazy transfers 37% (link) and 28% (style) fewer startup bytes
  on a warm visit, because the menus' templates are no longer in the HTML. Cold
  startup bytes are about the same: 460 and 455 bytes fewer out of about 266 KB.
- **Speculative loads.** Before `load`, lazy fetched only packs at low priority,
  and it fetched nothing while the page was prerendering. The component-asset
  loader in this benchmark fetched scripts at high priority and stylesheets at
  very high priority when the user hovered before `load`.
- **Against eager** (2026-10-29). Lazy hydrated 281 ms earlier, painted LCP
  172 ms earlier, transferred 14.7 KB less, and started with 90 KiB less heap than
  a page that puts everything in the startup bundle.

Where it is even:

- **Startup blocking time and FCP.** Lazy's startup TBT is now 0 ms at the median,
  as for component assets. It was 9 to 15 ms before the initial-focus prefetch
  moved after `load` and idle.
- **Hover opens.** Hovers that move within one shadow tree now prefetch, so
  settings opens as fast as with component assets wherever the pointer comes from
  (117 to 146 ms against 122 to 146 ms).
- **LCP in style builds.** +2 [-16, 24] ms.

Where it does worse:

- **Startup heap.** Lazy starts with 46 to 48 KiB more JS heap than component
  assets, as before. On 2026-10-29, 34 KiB of it was the autosuggest pack that the
  autofocused search box loads and 13 KiB the lazy runtime. The pack now loads
  after `load` at low priority, but still before the startup reading two seconds
  later. Loading autosuggest early is a trade-off, since it spares the first
  suggestion a round trip.
- **LCP in link builds.** The rerun's +20 [-4, 36] ms passes the criterion only
  because 20 samples give a wide interval; the full run measured +18 [4, 22] ms on
  60 samples. We have not traced it.
- **Menus opened by click alone** (2026-10-29). A menu opened without a hover or
  prefetch waited one round trip for its pack: 206 to 212 ms against 32 to 34 ms,
  because component assets keep the menus in the startup page. A hover or
  `$prefetch()` removes the wait.
- **Memory after use.** After three cycles of all ten UIs, lazy holds 2% more heap
  than component assets, not 10% less. Releasing closed regions works, since no
  region stayed loaded, but it doesn't win back the startup difference: lazy is
  45 to 46 KiB above component assets at startup and 54 to 58 KiB above after the
  third cycle. Both on-demand designs ended above eager-link (2,262 KiB on
  2026-10-29) after use.

In short, at `d6970af6` the new design meets the targets for FCP, startup
blocking time, startup bytes, hover opens, prerendering, and request priority, and
at `f4e24c25` it met them for code-prefetch and click-only opens. It misses the
startup heap target (+46 to +48 KiB) and the memory target (2% above component
assets instead of at least 10% below). LCP passes the criterion in both link and
style builds in the rerun, but the link-build gap of about 20 ms remains.

## Prefetch triggers and pay-for-play

`triggers.mjs` compiles one app five times: an ordinary `<if>` (**plain**) and
`<if lazy="demand|intent|idle|eager">`. Lazy variants import the generated
`lazy.webui.js`, as applications do: a first `build()` writes it and
`lazy.webui.d.ts` to `.webui/`, and the driver checks that the final build
generates the same module. Each app has a grid of cards; each card owns a native
`<dialog>` whose 24-field form, repeat, and nested component live in the body.
**s1** has one dialog; **s30** has 30, and the user opens one. Opening sets the
condition, calls `showModal()` once the body has rendered (lazy pages await
`this.$rendered()`; plain pages, which do not load the lazy runtime, flush
updates), and waits for paint.

The run uses Edge with the Lighthouse mobile profile (150 ms RTT, 1.6 Mbps down,
750 Kbps up, 4x CPU), 20 samples after one warmup in alternating order, and fresh
browser contexts. **Cold** has an empty HTTP cache; **warm** first visits the page
and opens the dialog in another tab, so scripts and packs come from the HTTP cache.
That tab is closed 1.5 seconds before the measured page loads: Chromium tears a
closed tab's compositor down about a second later on the renderer thread that
same-origin pages share, which otherwise adds hundreds of milliseconds to an
open that coincides with it. The driver opens the dialog 300 ms after hydration;
for intent, it hovers for 200 ms before clicking. Idle and eager count as started
only after every pack is registered. Static scripts and packs use Brotli 11 and
immutable caching; HTML uses Brotli 5 and `no-store`. Memory runs are
unthrottled: open and close all 30 dialogs three times, wait for idle release,
force GC, and read the JS heap. Output: `target/lazy-triggers.json` with every
sample, including each open's input delay, pack wait, render, `showModal()`, and
paint segments.

### Post-fix demand check: 2026-10-30

Source `d6970af6`, Edge 156.0.4314.58, with the machine, Node, and addon of the
NTP rerun. This check is bounded to plain and demand, cold, s1 and s30, with 20
samples after one warmup and no memory runs. It started about a minute after the
NTP rerun finished and took 4 minutes, with no failures. Startup JavaScript:
plain 26,242 / 26,846 bytes for s1 / s30, demand 28,556 / 29,114 (+2,314 /
+2,268).

| Scenario | Variant | Cold startup bytes | Startup heap | Startup TBT | Cold open (p90) | Pack wait | Opens over 400 ms |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| s1 | plain | 29,225 | 1,456 KiB | 0 ms | 83 (88) ms | - | 0 of 20 |
| s1 | demand | 30,672 | 1,459 KiB | 0 ms | 489 (561) ms | 180 ms | 11 of 20 |
| s30 | plain | 36,434 | 2,124 KiB | 11 ms | 81 (96) ms | - | 0 of 20 |
| s30 | demand | 33,474 | 1,871 KiB | 11 ms | 284 (583) ms | 183 ms | 10 of 20 |

The slow early demand opens described under the 2026-10-29 result came back:
11 of 20 s1 and 10 of 20 s30 demand opens took 447 to 689 ms, and the rest took
256 to 284 ms. The signature is the same as before. The pack arrived on time
(174 to 192 ms into the open), and the extra 180 to 420 ms passed without more
reported JavaScript Long Task blocking than a normal open (at most 30 ms), either between the
pack's arrival and `$rendered()` (16 opens) or in the frame after `showModal()`
(5 opens). The target from the 2026-10-27 recording, no demand regression beyond
the larger runtime, fails: demand opened in 308 (337) and 311 (339) ms then,
against 489 (561) and 284 (583) ms now.

An instrumented investigation after the 2026-10-29 run found that those outliers
clustered 1.20 to 1.75 seconds after navigation, both before the task that
consumed the pack and after `showModal()`, and that plain opens paused at the
same time. It found no runtime wait and no connection contention in the
instrumented path. Its fresh runs on Edge 156.0.4314.58 had no stall in 160
opens, 100 of them demand, with demand medians of 268 to 273 ms and p90s of 276
to 290 ms. Those runs used a different, earlier build than the committed source
(demand startup scripts of 28,550 / 29,135 bytes, against 28,556 / 29,114 here),
so they do not show that the committed source avoids the stall. A subsequent
repeat with the committed script sizes also had no stalls in 80 opens (demand
p90 283 / 283 ms). Neither clean run establishes that the tail is fixed.

### Native trace diagnosis: 2026-10-30

Tracing subsequently captured the pause on the committed `d6970af6` build in
Edge 156.0.4314.58. The s1 application was unchanged: 28,556 Brotli bytes,
SHA-256 `4d1befd1cb188c30f883da79e84830f7023bec67a733c42841fa23e5a879433f`.
The probe added no application instrumentation; it recorded native `toplevel`,
`cc`, `viz`, and renderer-scheduler events from before navigation through the
first open.

| Sample | Input-to-paint | Native cleanup starts after navigation | Native cleanup duration |
| --- | ---: | ---: | ---: |
| 1 | 513 ms | 1,186 ms | 243 ms |
| 2 | 610 ms | 1,217 ms | 338 ms |
| 3 | 609 ms | 1,195 ms | 348 ms |
| 4 | 284 ms | 1,258 ms | 4 ms |

All three slow samples are blocked in a retired browser widget's native
compositor teardown:
`WidgetBase::Shutdown` -> `LayerTreeHost::~LayerTreeHost` ->
`ProxyMain::Stop`. The renderer waits for the compositor's
`ProxyImpl::FinishGLOnImplThread`, which waits on GPU work. For the 338 ms
sample, `ProxyMain::Stop` used only 0.279 ms of renderer-thread CPU time.
The pack callback cannot run until this native task completes. This is not a
WebUI script/resource readiness wait or HTTP connection contention.

A control with the same application hash reproduced long cleanup under both
the workspace temporary directory and the operating system's default temporary
directory, using the same Intel/ANGLE D3D11 backend. The profile path alone
does not explain it. The GPU/driver condition that makes completion vary is
not isolated, and no browser fix is claimed.

The measured `renderMs` segment is elapsed time after the response, not a CPU
profile: it can include native browser waits. Long Task/TBT observations alone
missed this one. Input-to-paint still includes the pause, and the failing
samples and tail-target result above are retained. No samples are subtracted,
no extra open delay is added, and no GPU or browser feature is disabled to make
the benchmark pass. No WebUI runtime workaround is warranted by these traces;
adoption still needs first-use measurements in the application's browser/GPU
environment.

### Recorded result: 2026-10-29

Windows 10.0.26300, Intel Core Ultra 9 285K, Edge 156.0.4314.50, Node 24.12.0,
release addon at `f4e24c25`. 26 minutes, with no failures. Medians unless noted;
open is input-to-paint, and startup TBT is the cold median through two seconds
after hydration.

Startup JavaScript, Brotli 11: plain 26,242 / 26,846 bytes for s1 / s30; lazy
variants 28,535 / 29,050 (**+2,293 / +2,204 bytes** for the lazy runtime and the
generated module). Each pack is about 1,150 bytes. Pages without lazy regions do
not load the lazy runtime.

| Scenario | Variant | Cold startup bytes | Startup heap | Startup TBT | Cold open (p90) | Pack wait | Warm open (p90) |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| s1 | plain | 29,225 | 1,456 KiB | 0 ms | 82 (91) ms | - | 79 (88) ms |
| s1 | demand | 30,651 | 1,458 KiB | 0 ms | 271 (521) ms | 182 ms | 95 (106) ms |
| s1 | intent | 30,647 | 1,460 KiB | 0 ms | 78 (89) ms | 0 | 108 (124) ms |
| s1 | idle | 32,071 | 1,513 KiB | 0 ms | 82 (87) ms | 0 | 112 (134) ms |
| s1 | eager | 32,071 | 1,512 KiB | 0 ms | 80 (91) ms | 0 | 87 (108) ms |
| s30 | plain | 36,434 | 2,124 KiB | 10 ms | 84 (93) ms | - | 109 (125) ms |
| s30 | demand | 33,410 | 1,869 KiB | 6 ms | 276 (645) ms | 179 ms | 108 (129) ms |
| s30 | intent | 33,409 | 1,882 KiB | 8 ms | 78 (87) ms | 0 | 126 (219) ms |
| s30 | idle | 76,307 | 2,265 KiB | 8 ms | 77 (85) ms | 0 | 111 (126) ms |
| s30 | eager | 76,307 | 2,263 KiB | 10 ms | 77 (90) ms | 0 | 108 (126) ms |

Warm startup transfers 9,315 bytes for s30 plain and about 4,087 for every lazy
variant (2,710 and about 1,840 for s1). Releasing regions after use, s30:

| Variant | Startup heap | After cycle 1 | After cycle 3 | Regions loaded after release |
| --- | ---: | ---: | ---: | ---: |
| plain | 2,097 KiB | 2,416 KiB | 2,429 KiB | - |
| demand | 1,842 KiB | 2,263 KiB | 2,299 KiB | 0 of 30 in 5/5 runs |

What this shows:

- **Pay for play.** With 30 dialogs that start closed, demand and intent transfer
  8.3% fewer startup bytes than plain, use 12.0% less startup heap, and transfer
  56% fewer bytes on a repeat visit. After three cycles of using all 30, the heap
  is 5.4% below plain because closed regions are released.
- **Prefetch hides the network.** Intent, idle, and eager opens wait 0 ms for packs
  and open as fast as plain: 77 to 82 ms against 82 and 84. Intent adds no
  startup bytes and 2 / 13 KiB of startup heap over demand, so it is the default.
- **Demand pays for the network.** A cold demand open takes 189 / 192 ms longer
  than plain at the median: 182 / 179 ms of pack wait, which is one round trip
  plus the transfer, and about 10 ms to install the pack (the render segment takes
  30 / 33 ms against 21 / 20). From the HTTP cache the pack arrives in 7 ms.
- **Prefetching everything is not free.** Idle or eager prefetch of all 30 regions
  transfers 2.1 times plain's startup bytes and retains 6.6% more startup heap.
  Use those triggers for the few regions a user is likely to open soon.
- **One small region costs more than it saves.** For s1, demand and intent add
  about 1,420 startup bytes and a request when the dialog opens, and save no
  startup heap.

Changes since the 2026-10-27 recording:

- The lazy runtime's cost over plain grew from 1,716 / 1,738 to 2,293 / 2,204
  bytes, with fetch priorities, prerendering support, `$prefetch()`, region
  scripts, and `lazyerror`. Plain's script grew 895 / 841 bytes.
- Every variant opened faster (plain cold 82 / 84 ms, against 135 / 145 ms for
  the variant then called eager), and s30 startup blocking time fell for every
  variant (plain 10 ms, against 41). Compare variants within one run.
- Demand's cold median fell from 308 / 311 ms to 271 / 276 ms, and its gap to
  plain grew from 173 / 166 ms to 189 / 192 ms. Its p90 rose from 337 / 339 ms to
  521 / 645 ms because of the slow early opens described next.

**Slow early demand opens.** 3 of 20 cold demand opens in s1 and 8 of 20 in s30
took 445 to 706 ms. In each, the pack arrived on time (its response ended 173 to
188 ms into the open), and JavaScript timing suggested idle time while the open waited. In a
rerun that recorded Long Animation Frames, the task that consumed the pack
started 250 to 420 ms after the response ended, with no long task or long frame
in between; in other samples, the frame after `showModal()` came 380 to 425 ms
later than usual. The driver opens 300 ms after hydration, about one second
after navigation. Opening 2 seconds after hydration removed the stall (0 of 20;
median 294 ms, p90 321). Skipping the forced GC before the click (4 of 20 stalled)
or waiting 1.5 seconds between samples (3 of 20) did not. When plain opens were
delayed to the same point after navigation, one in 20 stalled, in the frame after
`showModal()`. We had not found the cause when this run was recorded. The recorded
run used the same Edge build as the 2026-10-27 recording, which did not show the
stall; the reruns used Edge 156.0.4314.56. The 2026-10-30 check above has the
later investigation and the result on the committed fix.

Warm opens of intent, idle, and eager are 7 to 48 ms slower than their cold
opens, and plain s30 is 25 ms slower. Those warm opens include 22 to 53 ms of
main-thread blocking against 13 to 20 ms cold. We have not traced this work; it
also affects plain s30, so it is not specific to lazy regions.

For comparison, an earlier run of this workload on the same machine measured the
previous endpoint design at 321 / 310 ms for a cold demand open (s1 / s30), with
4.4 KB more startup JavaScript than plain. Its start-time preload of all 30
regions added 107 ms of startup blocking time; packs now install one per network
task.

## Startup, open, and retention by body size

`bench.mjs` compares one owner with 25, 250, or 1,000 controls in a lazy body:

- **eager-local**: ordinary `<if>`, templates and state already local; no open request.
- **eager-fresh**: ordinary `<if>`, templates local, fresh state fetched on every open.
- **lazy**: `<if lazy="demand">`; opening starts the state request, sets the
  condition so the pack downloads at the same time, then applies the state and
  awaits `this.$rendered()`.

`WEBUI_BENCH_SAMPLES` defaults to 20 (plus two discarded warmups);
`WEBUI_BENCH_LATENCY_MS` defaults to 40 ms of state-provider delay. Fresh browser
contexts alternate execution order. Heap readings follow forced GC; closed-body
weak-reference results are reported separately. Output:
`target/lazy-browser-proof.json`.

### Recorded result: 2026-10-28

Same machine and addon, Playwright Chromium. Medians.

| Controls | Mode | Startup HTML + JS gzip | Initial heap | Cold open | Warm open | Closed body retained |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 25 | eager-fresh | 21,837 | 1,859,348 | 51.8 ms | 55.5 ms | 20/20 |
| 25 | lazy | 23,721 | 1,857,248 | 47.2 ms | 50.7 ms | 0/20 |
| 250 | eager-fresh | 23,556 | 1,887,480 | 53.2 ms | 51.0 ms | 20/20 |
| 250 | lazy | 23,722 | 1,857,248 | 48.1 ms | 54.4 ms | 0/20 |
| 1,000 | eager-fresh | 29,317 | 1,980,568 | 53.7 ms | 50.7 ms | 20/20 |
| 1,000 | lazy | 23,719 | 1,857,248 | 45.0 ms | 50.2 ms | 0/20 |

Lazy startup no longer grows with the body. The fixed cost at 25 controls is
1,884 gzip bytes, against 4,152 with the earlier endpoint design; applications
without lazy regions pay nothing, because the rendering core carries no lazy
code. Lazy delivery breaks even just above 250 controls (166 bytes more at 250),
and 1,000 controls save 19% of startup bytes and 123 KB of heap. Cold opens are
4-9 ms faster than eager-fresh because the lazy variant sets its condition before
awaiting state, so the body renders while the state request is in flight;
eager-fresh renders only after its state arrives. Eager-local, which requests
nothing, opens in 1.1-4.6 ms.

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
