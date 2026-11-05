# Framework render benchmark

Contributor tooling, not an application API.

From `packages/webui-framework`, start `node --experimental-strip-types
tests/server.ts`, then run `pnpm bench:render <baseline-ref> [comparison-ref]`
in a second terminal. Build the native addon first; set `WEBUI_ADDON_PATH` when
testing a specific build.

The benchmark bundles complete framework source snapshots with identical
production optimization and fixture HTML. Measurements rotate across snapshots
and verify nested DOM output. JSON output includes 30 samples, P50/P95,
fixture bundle sizes, and GC-collected heap usage. `retainedHeap` includes full
snapshot totals, node-type sizes, and external-string storage, including script
source. `retainedTree` and the churn `memory` series retain the VM-used-size
metric for comparison with older runs; that metric alone omits external source.

Normal-update measurements use equal microtask turns for this three-level
fixture; they are not an application completion recipe. Cached manual flushing
is a historical baseline. Heap measurements include fixture/browser state,
not just framework allocations. Run without other builds or tests and compare
multiple runs before drawing conclusions.

For a strict heap comparison, use fresh contexts and independently launched
browsers, rotate case order, settle resources and two frames before full GC, and
compare full snapshot totals rather than only live objects or VM-used size.
Nonempty stylesheets must be applied in both versions before sampling; Module
CSS also requires preserved import attributes and browser support.

Set `BENCH_BINDING=complex` to measure two complex property bindings per child
instead of the default reflected-attribute binding. Both profiles verify the
final descendant DOM; the complex profile also verifies the second property.
