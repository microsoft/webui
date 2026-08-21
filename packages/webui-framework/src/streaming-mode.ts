// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Streaming-hydration mode detection.
 *
 * Kept in its own leaf module (no imports) so `template-element.ts` can check
 * `isStreamingHydrationMode()` without importing the coordinator
 * (`streaming.ts`), which itself imports `static-host.ts` →
 * `template-element.ts`. A direct import from `template-element.ts` back to
 * `streaming.ts` would close that cycle.
 *
 * It carries only what the always-shipped bundle genuinely needs: mode
 * detection, the two shared hook symbols, the single `data-ws` dormancy
 * marker, and the numeric activation-outcome codes both sides of the
 * `STREAMING_BOUNDARY_ACTIVATE` contract speak. Everything span-shaped — the
 * `data-ws-span` / `data-ws-enclosing` attribute names and the open-span
 * registry that resolves them — lives in the opt-in streaming graph
 * (`streaming-dom.ts`, `streaming-spans.ts`), so a non-streaming app never
 * downloads a byte of it.
 */

let cached: boolean | undefined;

/** Shared activation hook installed on streamed template elements. */
export const STREAMING_BOUNDARY_ACTIVATE = Symbol.for(
  'microsoft.webui.boundaryActivate',
);
/** Shared resume hook for definition and ancestor-barrier deferred roots. */
export const PENDING_ROOT_CONNECTED = Symbol.for(
  'microsoft.webui.pendingRootConnected',
);
/** Compiler-owned marker for an uncommitted streamed host. */
export const STREAMED_HOST_ATTR = 'data-ws';

// ── Activation outcomes ─────────────────────────────────────────
//
// The producer (`template-element.ts`) and the consumer
// (`streaming-deferred.ts`) both import these, so the contract has exactly one
// definition and `ActivationOutcome` makes TypeScript reject a producer that
// invents a code the consumer cannot decode.
//
// Numeric codes rather than result objects: the coordinator classifies every
// marked root on the boundary walk, and a per-root object there would allocate
// once per streamed component.
//
// The range is `1..4` on purpose. The coordinator's own element-walk results
// (`ELEMENT_*` in `streaming-deferred.ts`) occupy a disjoint decade, so the two
// spaces can flow through one `number` without a code from either side ever
// being mistaken for the other. Codes are also non-zero so a hook that returns
// nothing is rejected rather than decoded.

/** The root hydrated, or was already live and needed no work. */
export const ACTIVATION_ACTIVATED = 1;
/** A compiler-owned static host declined activation on purpose. */
export const ACTIVATION_STATIC_HOST_OPT_OUT = 2;
/** Template metadata was unavailable; the root cannot hydrate. */
export const ACTIVATION_MISSING_TEMPLATE = 3;
/** An unfinished ancestor owns this root until its barrier lifts. */
export const ACTIVATION_ANCESTOR_BARRIER = 4;

/**
 * Every code `STREAMING_BOUNDARY_ACTIVATE` is allowed to return.
 *
 * Typing the hook with this union is what enforces producer/consumer parity at
 * compile time. It deliberately does not describe what a *foreign* element may
 * hand back at runtime: the coordinator still validates the returned value and
 * fails closed on anything outside this set.
 */
export type ActivationOutcome =
  | typeof ACTIVATION_ACTIVATED
  | typeof ACTIVATION_STATIC_HOST_OPT_OUT
  | typeof ACTIVATION_MISSING_TEMPLATE
  | typeof ACTIVATION_ANCESTOR_BARRIER;

/**
 * Whether this document was served in streaming-hydration mode.
 *
 * Detected once via a single `<meta name="webui-streaming" content="1">`
 * query and cached for the lifetime of the document — callers on the hot
 * hydration path (every `TemplateElement.connectedCallback`) never repeat
 * the query.
 */
export function isStreamingHydrationMode(): boolean {
  if (cached !== undefined) return cached;
  cached = typeof document !== 'undefined' &&
    !!document.querySelector('meta[name="webui-streaming"][content="1"]');
  return cached;
}

/** Test-only: reset the cached detection result. */
export function resetStreamingModeForTests(): void {
  cached = undefined;
}
