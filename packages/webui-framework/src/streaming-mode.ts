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
/**
 * Compiler-owned SpanInstanceId on an unfinished component host.
 *
 * The value is a canonical base-10 integer. It identifies the root-local
 * `<!--ws:S-->...<!--/ws:S-->` range that will eventually activate this host.
 */
export const STREAMING_SPAN_HOST_ATTR = 'data-ws-span';
/**
 * Compiler-owned enclosing SpanInstanceId on an early boundary child root.
 *
 * Matching this value to `data-ws-span` lets that root bypass exactly one
 * unfinished ancestor barrier. Unmarked or mismatched roots stay dormant.
 */
export const STREAMING_ENCLOSING_SPAN_ATTR = 'data-ws-enclosing';

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
