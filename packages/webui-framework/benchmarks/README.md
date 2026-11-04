# Framework render benchmark

Contributor tooling, not an application API.

From `packages/webui-framework`, start `node --experimental-strip-types
tests/server.ts`, then run `pnpm bench:render <baseline-ref> [comparison-ref]`
in a second terminal. Build the native addon first; set `WEBUI_ADDON_PATH` when
testing a specific build.

The benchmark bundles complete framework source snapshots with identical
production optimization and fixture HTML. Measurements rotate across snapshots
and verify nested DOM output. JSON output includes 30 samples, P50/P95,
fixture bundle sizes, and GC-collected heap usage.

Normal-update measurements use equal microtask turns for this three-level
fixture; they are not an application completion recipe. Cached manual flushing
is a historical baseline. Heap measurements include fixture/browser state,
not just framework allocations. Run without other builds or tests and compare
multiple runs before drawing conclusions.
