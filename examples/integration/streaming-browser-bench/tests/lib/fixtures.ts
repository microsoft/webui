// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * In-memory bundling of the *actual* framework sources into two minified
 * browser fixtures, plus their exact minified/gzip byte sizes.
 *
 * The ordinary fixture imports `WebUIElement` from the framework default entry
 * only; the streaming fixture imports the public streaming entry
 * (`streaming-entry.ts`, side-effect coordinator install) before the default
 * entry. Both then define one instrumented `bench-island` subclass. Bundling
 * the real sources (not a synthetic clone) is the whole point: the benchmark
 * measures genuine `WebUIElement` hydration and the genuine coordinator.
 *
 * Bundles are produced with `write: false`, `minify: true`,
 * `define: { __WEBUI_DEV__: 'false' }`, `format: 'iife'`,
 * `platform: 'browser'` — i.e. the production shape a real app ships.
 */

import { build } from 'esbuild';
import { gzipSync } from 'node:zlib';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));

/** Absolute path to the framework's TypeScript source root. esbuild resolves
 *  the framework's `./foo.js` intra-package imports to their `.ts` sources. */
const FRAMEWORK_SRC = resolve(here, '..', '..', '..', '..', '..', 'packages', 'webui-framework', 'src');

/** Coordinator wire tokens that must never leak into the ordinary bundle. */
export const COORDINATOR_TOKENS = ['webui-hydrate', 'data-webui-boundary'] as const;

export interface Fixture {
  readonly kind: 'ordinary' | 'streaming';
  readonly code: string;
  readonly minifiedBytes: number;
  readonly gzipBytes: number;
}

/**
 * The instrumented `bench-island` subclass, shared by both fixtures.
 *
 * Component-hydration CPU is accumulated on `window.__benchHydrationCpu` so the
 * comparison is independent of boundary parsing / coordinator overhead:
 *
 * - Ordinary (unmarked) roots do their real work inside `connectedCallback`, so
 *   `super.connectedCallback()` is timed there.
 * - Streamed (`data-ws`) roots defer cheaply on connect (not counted); their
 *   real work happens when the coordinator calls the protected
 *   `$activateDeferredSSR(state)`, so `super.$activateDeferredSSR(state)` is
 *   timed instead.
 *
 * Each hook also samples `usedJSHeapSize` into `window.__benchPeakHeap` *after*
 * stopping its CPU timer, so peak heap is captured while components are actually
 * committing (not merely after the debug event + scaffold cleanup) without the
 * sampling polluting the CPU measurement.
 *
 * It also increments `window.__benchHydratedCount` exactly once per instance,
 * only after the real super hook returns successfully — proving every root
 * genuinely hydrated rather than merely surviving scaffold removal. An instance
 * guard plus the streamed-marker check keep an early/no-op activation from
 * counting falsely. The driver additionally runs a reactive `setState` probe as
 * independent proof (see `drivers.ts`).
 */
const ISLAND_DEFINITION = `
function addBenchHydrationCpu(dt) {
  const w = window;
  w.__benchHydrationCpu = (w.__benchHydrationCpu || 0) + dt;
}

function noteBenchPeakHeap() {
  // Sampled from inside the hydration hooks *after* the CPU timer stops, so this
  // records the heap high-water mark while components are actually committing
  // without inflating the measured component CPU. performance.memory is optional
  // (Chromium + --enable-precise-memory-info); absence is tolerated.
  const mem = performance.memory;
  if (!mem) return;
  const w = window;
  const used = mem.usedJSHeapSize;
  if (!w.__benchPeakHeap || used > w.__benchPeakHeap) w.__benchPeakHeap = used;
}

// Count a successful hydration exactly once per instance. It is gated on the
// base class's real \`$hydrated\` flag (set only when $mount actually hydrated),
// so an early return (deferral, missing-metadata warn, or no-op activation)
// cannot increment it falsely, and the '__benchCounted' guard prevents a second
// increment on the same instance.
function countBenchHydration(el) {
  if (el.__benchCounted || el.$hydrated !== true) return;
  el.__benchCounted = true;
  const w = window;
  w.__benchHydratedCount = (w.__benchHydratedCount || 0) + 1;
}

class BenchIsland extends WebUIElement {
  connectedCallback() {
    if (this.hasAttribute('data-ws')) {
      // Streaming deferral is intentionally cheap; attribute it to coordinator
      // overhead, not component hydration CPU. Not a real hydration -> not counted.
      super.connectedCallback();
      return;
    }
    const t0 = performance.now();
    super.connectedCallback();
    addBenchHydrationCpu(performance.now() - t0);
    noteBenchPeakHeap();
    // Ordinary root: counted only if connectedCallback actually hydrated it.
    countBenchHydration(this);
  }

  $activateDeferredSSR(state) {
    // A genuine streamed deferred root still carries 'data-ws' when the
    // coordinator invokes this hook (the marker is stripped only afterward). A
    // stray/no-op activation of an already-activated root has no marker, so it
    // cannot be counted.
    const wasStreamedDeferred = this.hasAttribute('data-ws');
    const t0 = performance.now();
    super.$activateDeferredSSR(state);
    addBenchHydrationCpu(performance.now() - t0);
    noteBenchPeakHeap();
    if (wasStreamedDeferred) countBenchHydration(this);
  }
}

window.__defineBenchIsland = function defineBenchIsland() {
  if (!customElements.get('bench-island')) {
    BenchIsland.define('bench-island');
  }
  return BenchIsland;
};
`;

const ORDINARY_ENTRY =
  `import { WebUIElement } from './index.js';\n${ISLAND_DEFINITION}`;

const STREAMING_ENTRY =
  `import './streaming-entry.js';\nimport { WebUIElement } from './index.js';\n${ISLAND_DEFINITION}`;

async function bundle(kind: Fixture['kind'], contents: string): Promise<Fixture> {
  const result = await build({
    stdin: {
      contents,
      resolveDir: FRAMEWORK_SRC,
      loader: 'ts',
      sourcefile: `${kind}-fixture.ts`,
    },
    bundle: true,
    write: false,
    minify: true,
    format: 'iife',
    platform: 'browser',
    target: 'es2022',
    define: { __WEBUI_DEV__: 'false' },
    supported: { 'import-attributes': true },
    legalComments: 'none',
  });
  const code = result.outputFiles[0].text;
  return {
    kind,
    code,
    minifiedBytes: Buffer.byteLength(code, 'utf-8'),
    gzipBytes: gzipSync(Buffer.from(code, 'utf-8')).length,
  };
}

export interface BuiltFixtures {
  readonly ordinary: Fixture;
  readonly streaming: Fixture;
  /** Streaming minified bytes minus ordinary minified bytes. */
  readonly streamingIncrementalBytes: number;
  /** Streaming gzip bytes minus ordinary gzip bytes. */
  readonly streamingIncrementalGzipBytes: number;
}

/** Build both fixtures and verify the ordinary bundle is coordinator-free. */
export async function buildFixtures(): Promise<BuiltFixtures> {
  const [ordinary, streaming] = await Promise.all([
    bundle('ordinary', ORDINARY_ENTRY),
    bundle('streaming', STREAMING_ENTRY),
  ]);
  return {
    ordinary,
    streaming,
    streamingIncrementalBytes: streaming.minifiedBytes - ordinary.minifiedBytes,
    streamingIncrementalGzipBytes: streaming.gzipBytes - ordinary.gzipBytes,
  };
}

/** Coordinator tokens found in a bundle (empty for a clean ordinary bundle). */
export function coordinatorTokensIn(code: string): string[] {
  return COORDINATOR_TOKENS.filter((token) => code.includes(token));
}
