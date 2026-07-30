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
/** Shared reconnect hook retained by a detached undefined streamed root. */
export const PENDING_ROOT_CONNECTED = Symbol.for(
  'microsoft.webui.pendingRootConnected',
);
/** Compiler-owned marker for an uncommitted streamed host. */
export const STREAMED_HOST_ATTR = 'data-ws';

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
