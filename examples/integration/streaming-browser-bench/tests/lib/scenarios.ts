// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Deterministic SSR scaffold generation for the progressive-hydration matrix.
 *
 * Every scenario renders the *same* fixed total number of real `bench-island`
 * SSR roots and the *same* fixed total state bytes; only the number of
 * streamed boundaries (and the marker layout) changes. That is what makes the
 * "equal total work" comparison fair: the per-boundary coordinator overhead is
 * the only variable, so component-hydration CPU and retained heap should stay
 * flat as the boundary count grows from 1 to 100.
 *
 * The markup emitted here is byte-for-byte the wire format the
 * coordinator (`packages/webui-framework/src/streaming.ts`) parses:
 *
 *   <!--wb:N--> <bench-island data-ws>...</bench-island> <!--/wb:N-->
 *   <script type="application/json" data-webui-boundary>[1,seq,terminal,bootstrap]</script>
 *   <webui-hydrate></webui-hydrate>
 *
 * No whitespace is emitted between the `<!--/wb:N-->` end marker and the
 * boundary `<script>`, nor between the script and the `<webui-hydrate>`
 * sentinel: the coordinator locates the payload via `previousSibling`
 * (exact-adjacency for the end marker) so an intervening text node would break
 * discovery. Roots may be separated by structure (the nested layout relies on
 * it) because start-marker discovery walks every previous sibling.
 */

/** Fixed total real SSR roots across every scenario (divisible by 1/3/10/100).
 *  Sized so a full matrix of real hydrations accumulates several milliseconds of
 *  component CPU, keeping the strict CPU gate above timer noise. */
export const TOTAL_ROOTS = 1500;

/**
 * Fixed total *projected state value* characters across every scenario.
 *
 * This counts only the bytes of the streamed `label` values (the dominant
 * projected state a real app would ship), summed across all boundaries. It
 * deliberately excludes the unavoidable per-boundary protocol/property overhead
 * — the `[1,seq,terminal,{...}]` envelope framing, the `templates` block (first
 * boundary only), and the tiny fixed `note` property — because that overhead is
 * inherent to having more boundaries and is not "equal work" to hold constant.
 * It is therefore projected-state value bytes, not total wire bytes.
 */
export const TOTAL_STATE_VALUE_BYTES = 24_000;

/** Boundary counts for the primary equal-total-work matrix. */
export const BOUNDARY_COUNTS = [1, 3, 10, 100] as const;

/** Custom-element tag every SSR root upgrades to. */
export const ISLAND_TAG = 'bench-island';

/**
 * Compiled template metadata for `bench-island`.
 *
 * Minimal but *real*, and deliberately more than a no-op: two child elements
 * (`<span>` / `<em>`) each with a text binding (reading the `label` and `note`
 * state roots), so activation exercises the genuine `$hydrate` text-binding path
 * (SSR text-node discovery + wiring) twice per root. `__WEBUI_DEV__=false` strips
 * the hydration-mismatch check, so the SSR text need not equal the streamed state.
 */
export const ISLAND_TEMPLATE = {
  h: '<span></span><em></em>',
  tr: ['label', 'note'],
  tx: [
    [[[0], 0], [['label']]],
    [[[1], 0], [['note']]],
  ],
} as const;

/** Marker layout for a boundary's SSR roots. */
export type MarkerLayout = 'flat' | 'nested';

/** When the `bench-island` class is registered relative to boundary delivery. */
export type DefineTiming = 'eager' | 'race';

export interface StreamingScenario {
  readonly label: string;
  readonly boundaryCount: number;
  readonly layout: MarkerLayout;
  readonly timing: DefineTiming;
  /** One HTML fragment per boundary (roots + envelope + sentinel), in order. */
  readonly boundaries: readonly string[];
  /** The terminal envelope fragment (no markers, empty bootstrap). */
  readonly terminal: string;
  /** Sum of streamed `label` value characters (asserted equal across cases). */
  readonly totalStateChars: number;
  /** Sum of live SSR roots (asserted equal across cases). */
  readonly totalRoots: number;
}

export interface OrdinaryScenario {
  readonly label: string;
  /** Inert `#webui-data` bootstrap script. This must be present *before* the
   *  framework bundle runs: the default entry schedules `installTemplateElementRuntime`
   *  on a macrotask, which lazily loads `#webui-data` and latches its
   *  "already loaded" guard. If the block were absent at that point the guard
   *  would poison, and the later block would never load — leaving roots
   *  un-hydrated. So it goes in the base document, not the runtime insert. */
  readonly bootstrapHtml: string;
  /** The SSR roots only, inserted at run time (after the baseline heap sample)
   *  so the empty->populated peak-heap transition matches the streaming arms.
   *  The 24KiB bootstrap value sits in the baseline for the control (a minor,
   *  documented asymmetry dwarfed by the ~1.8MiB root working set). */
  readonly rootsHtml: string;
  readonly totalStateChars: number;
  readonly totalRoots: number;
}

/** Distribute `total` across `parts` buckets, spreading the remainder to the
 *  leading buckets so the sum is exactly `total` and buckets differ by <=1. */
function distribute(total: number, parts: number): number[] {
  const base = Math.floor(total / parts);
  const remainder = total - base * parts;
  const out = new Array<number>(parts);
  for (let i = 0; i < parts; i++) out[i] = base + (i < remainder ? 1 : 0);
  return out;
}

/** A single SSR host root carrying the streamed-host `data-ws` marker. */
function streamedRoot(cell: number): string {
  return `<${ISLAND_TAG} data-ws><span>${cell}</span><em>n</em></${ISLAND_TAG}>`;
}

/** A single SSR host root for the non-streaming control (no `data-ws`). */
function ordinaryRoot(cell: number): string {
  return `<${ISLAND_TAG}><span>${cell}</span><em>n</em></${ISLAND_TAG}>`;
}

/** Flat layout: all roots as direct siblings inside the marker range. */
function flatRoots(startCell: number, count: number): string {
  let html = '';
  for (let i = 0; i < count; i++) html += streamedRoot(startCell + i);
  return html;
}

/** Deeply nested layout: one root at each successive `<div>` depth, i.e.
 *  `<div>root0<div>root1<div>root2 ... </div></div></div>`, so the coordinator's
 *  pre-order walk must descend the whole tree to reach every root. Each root
 *  really does sit one level deeper than the previous one. */
function nestedRoots(startCell: number, count: number): string {
  let open = '';
  let close = '';
  for (let i = 0; i < count; i++) {
    open += `<div>${streamedRoot(startCell + i)}`;
    close += '</div>';
  }
  return open + close;
}

/** Build one streamed boundary fragment. `templates` is emitted only in the
 *  first boundary; state (`label` value + fixed `note`) is emitted in every
 *  boundary. `labelChars` is the projected-state value size for this boundary. */
function boundaryFragment(
  seq: number,
  cellStart: number,
  rootCount: number,
  label: string,
  layout: MarkerLayout,
  withTemplates: boolean,
): string {
  const bootstrap: Record<string, unknown> = {
    state: { label, note: 'n' },
  };
  if (withTemplates) bootstrap.templates = { [ISLAND_TAG]: ISLAND_TEMPLATE };
  const envelope = JSON.stringify([1, seq, 0, bootstrap]);
  const roots = layout === 'flat'
    ? flatRoots(cellStart, rootCount)
    : nestedRoots(cellStart, rootCount);
  return `<!--wb:${seq}-->${roots}<!--/wb:${seq}-->`
    + `<script type="application/json" data-webui-boundary>${envelope}</script>`
    + `<${'webui-hydrate'}></${'webui-hydrate'}>`;
}

/** The terminal envelope: no markers, `terminal = 1`, empty bootstrap. */
function terminalFragment(seq: number): string {
  const envelope = JSON.stringify([1, seq, 1, {}]);
  return `<script type="application/json" data-webui-boundary>${envelope}</script>`
    + `<${'webui-hydrate'}></${'webui-hydrate'}>`;
}

/** Build a full streamed scenario for a boundary count / layout / timing. */
export function buildStreamingScenario(
  boundaryCount: number,
  layout: MarkerLayout,
  timing: DefineTiming,
): StreamingScenario {
  const rootsPer = distribute(TOTAL_ROOTS, boundaryCount);
  const labelPer = distribute(TOTAL_STATE_VALUE_BYTES, boundaryCount);
  const boundaries: string[] = [];
  let cell = 0;
  let totalStateChars = 0;
  let totalRoots = 0;
  for (let seq = 0; seq < boundaryCount; seq++) {
    boundaries.push(
      boundaryFragment(
        seq,
        cell,
        rootsPer[seq],
        'x'.repeat(labelPer[seq]),
        layout,
        seq === 0,
      ),
    );
    cell += rootsPer[seq];
    totalStateChars += labelPer[seq];
    totalRoots += rootsPer[seq];
  }

  return {
    label: `${boundaryCount}-boundary ${layout} ${timing}`,
    boundaryCount,
    layout,
    timing,
    boundaries,
    terminal: terminalFragment(boundaryCount),
    totalStateChars,
    totalRoots,
  };
}

/** Build a correctness-only scenario with distinct state per boundary.
 * `rootsPerBoundary` makes the real hydration work large enough to remain
 * observable without changing the measured primary matrix. */
export function buildStateDeliveryScenario(
  labels: readonly string[],
  rootsPerBoundary: number,
): StreamingScenario {
  const boundaries = new Array<string>(labels.length);
  let cell = 0;
  let totalStateChars = 0;
  for (let seq = 0; seq < labels.length; seq++) {
    const label = labels[seq];
    boundaries[seq] = boundaryFragment(
      seq,
      cell,
      rootsPerBoundary,
      label,
      'flat',
      seq === 0,
    );
    cell += rootsPerBoundary;
    totalStateChars += label.length;
  }
  return {
    label: 'boundary-state delivery probe',
    boundaryCount: labels.length,
    layout: 'flat',
    timing: 'eager',
    boundaries,
    terminal: terminalFragment(labels.length),
    totalStateChars,
    totalRoots: labels.length * rootsPerBoundary,
  };
}

/** Build the non-streaming one-shot control (real inert `#webui-data`). The
 *  bootstrap script is delivered in the base document (see `bootstrapHtml` /
 *  `OrdinaryScenario`), while only the SSR roots are inserted at run time so the
 *  peak-heap transition matches the streaming arms. */
export function buildOrdinaryScenario(): OrdinaryScenario {
  const bootstrap = {
    templates: { [ISLAND_TAG]: ISLAND_TEMPLATE },
    state: { label: 'x'.repeat(TOTAL_STATE_VALUE_BYTES), note: 'n' },
  };
  const bootstrapHtml =
    `<script id="webui-data" type="application/json">${JSON.stringify(bootstrap)}</script>`;
  let rootsHtml = '';
  for (let i = 0; i < TOTAL_ROOTS; i++) rootsHtml += ordinaryRoot(i);
  return {
    label: 'ordinary one-shot control',
    bootstrapHtml,
    rootsHtml,
    totalStateChars: TOTAL_STATE_VALUE_BYTES,
    totalRoots: TOTAL_ROOTS,
  };
}

/** Base document for a streaming run. Two things must be true before the
 *  coordinator bundle runs:
 *  1. the `webui-streaming` meta is present so `isStreamingHydrationMode()` latches;
 *  2. `document.readyState` reports `'loading'`. The coordinator's truncation
 *     guard halts the stream if it installs on an already-finished document
 *     (`readyState !== 'loading'` schedules an immediate halt microtask). Because
 *     this harness injects the bundle after `setContent` completes and then
 *     streams boundaries from script, we pin `readyState` to `'loading'` so the
 *     guard instead waits for a real `DOMContentLoaded` that never fires — the
 *     terminal envelope, not the guard, ends the stream. `readyState` is read
 *     nowhere in the hydration path, only by that guard and a dormant static-host
 *     listener, so the override is inert elsewhere.
 *  The body starts empty; boundaries are appended one at a time and the driver
 *  synchronises on deterministic DOM scaffolding removal. */
export function streamingBaseHtml(): string {
  return '<!doctype html><html><head><meta charset="utf-8">'
    + '<meta name="webui-streaming" content="1">'
    + '<script>Object.defineProperty(document,"readyState",{configurable:true,get:function(){return "loading";}});</script>'
    + '</head><body></body></html>';
}

/** Base document for the ordinary control: no streaming meta. The inert
 *  `#webui-data` bootstrap lives in the body from the start (so the framework's
 *  lazy loader latches on the real block, not an empty DOM); the SSR roots are
 *  inserted at run time, mirroring how the streaming arms start rootless. */
export function ordinaryBaseHtml(bootstrapHtml: string): string {
  return '<!doctype html><html><head><meta charset="utf-8"></head>'
    + `<body>${bootstrapHtml}</body></html>`;
}
