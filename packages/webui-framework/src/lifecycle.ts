// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Hydration lifecycle tracker.
 *
 * Tracks aggregate hydration timing via the Performance API and fires a
 * global `webui:hydration-complete` event on `window` once the startup
 * hydration cohort has settled. Visibility-deferred lazy components do not
 * keep this one-shot event open indefinitely.
 *
 * ## Performance marks
 *
 * Global:
 * - `webui:hydrate:total:start`  — first component begins hydrating
 * - `webui:hydrate:total:end`    — startup cohort settles
 * - measure `webui:hydrate:total`
 *
 * ## Window event
 *
 * `webui:hydration-complete` — dispatched once on `window` when the startup
 * cohort is hydrated or classified as visibility-dormant.
 *
 * ## Streaming gate
 *
 * A streaming-hydration page commits components across several boundaries
 * instead of all at once. `pendingCount` alone would let
 * `webui:hydration-complete` fire the instant an early boundary's components
 * finish, even though later boundaries (and the terminal record) haven't
 * arrived yet. The mode marker reserves that gate even when the application
 * arrives first. The streaming coordinator (`streaming.ts`) also opens it
 * with `beginStreamingGate()` and reports boundary lifecycle with
 * `markBoundaryPending()` / `markBoundaryCommitted()`. On non-streaming pages
 * the gate stays inactive and behavior is unchanged.
 *
 * Streamed SSR hosts carry a compiler-owned `data-ws` identity, so the
 * completion gate does not need to expose parser-window state to components.
 */

import { isStreamingHydrationMode } from './streaming-mode.js';

/** How many startup or active-batch hydration operations remain. */
let pendingCount = 0;

/** Whether the global start mark has been placed. */
let started = false;

/** Whether the global complete event has already fired. */
let completed = false;

/** Ordinary startup does not complete before parser-loaded components register. */
let documentReady = true;
let lifecycleInstalled = false;
let hasBrowserDocument = false;

/** Install parser-readiness tracking explicitly, without side effects from API imports. */
export function installHydrationLifecycle(): void {
  if (lifecycleInstalled) return;
  lifecycleInstalled = true;
  hasBrowserDocument =
    typeof document !== 'undefined' &&
    typeof Document !== 'undefined' &&
    document instanceof Document;
  const navigationTiming = hasBrowserDocument &&
    typeof performance.getEntriesByType === 'function'
    ? performance.getEntriesByType('navigation')[0] as PerformanceNavigationTiming | undefined
    : undefined;
  documentReady = !hasBrowserDocument ||
    document.readyState === 'complete' ||
    (navigationTiming?.domContentLoadedEventStart ?? 0) > 0;
  if (documentReady) return;
  const markDocumentReady = (): void => {
    if (documentReady) return;
    documentReady = true;
    tryComplete();
  };
  document.addEventListener('DOMContentLoaded', markDocumentReady, { once: true });
  // A late-loaded module can miss DOMContentLoaded when navigation timing is
  // unavailable. `load` is the next authoritative signal and cannot race
  // parser module startup.
  window.addEventListener('load', markDocumentReady, { once: true });
}

/** Whether parser/deferred module startup can still register hydration work. */
export function isHydrationStartupPending(): boolean {
  return !documentReady;
}

const enum StreamingStatus {
  Terminal = 1,
  Aborted = 2,
}

const enum StreamingField {
  Status,
  Boundaries,
  LateActivations,
}

/** Compact counters share Array storage without a page-specific object shape. */
type StreamingGate = [status: number, boundaries: number, lateActivations: number];

/** Ordinary hydration owns no streaming completion state. */
let streamingGate: StreamingGate | undefined;

function requireStreamingGate(): StreamingGate {
  return streamingGate ??= [0, 0, 0];
}

/**
 * Call before a component begins hydration.
 * Increments the pending counter and (once) places the global start mark.
 */
export function hydrationStart(): void {
  if (!started) {
    performance.mark('webui:hydrate:total:start');
    started = true;
  }
  pendingCount++;
}

/**
 * Call after a component has finished hydration.
 * When the last component finishes, fires the global event + measure.
 */
export function hydrationEnd(): void {
  if (pendingCount === 0) return;
  pendingCount--;
  tryComplete();
}

/** Keep synchronous definition batches in one cohort without starting a timing mark. */
export function runHydrationBatch<T>(work: () => T): T {
  pendingCount++;
  try {
    return work();
  } finally {
    hydrationEnd();
  }
}

/** Opt this page into the streaming-aware completion gate. Idempotent. */
export function beginStreamingGate(): void {
  requireStreamingGate();
}

/**
 * Abort the streaming gate: the coordinator hit an unrecoverable failure, so
 * `webui:hydration-complete` must never fire, even as failure cleanup settles
 * any outstanding boundary/late-activation counters. Idempotent and one-way.
 * A completion that already dispatched before the abort is unaffected (the
 * `completed` latch prevents a second dispatch either way).
 */
export function abortStreamingGate(): void {
  requireStreamingGate()[StreamingField.Status] |= StreamingStatus.Aborted;
}

/** Call when a streamed boundary begins committing its roots. */
export function markBoundaryPending(): void {
  requireStreamingGate()[StreamingField.Boundaries]++;
}

/**
 * Call when a streamed boundary finishes committing its roots.
 * `terminal` is true for the boundary carrying the terminal record.
 */
export function markBoundaryCommitted(terminal: boolean): void {
  const gate = streamingGate;
  if (!gate || gate[StreamingField.Boundaries] === 0) return;
  gate[StreamingField.Boundaries]--;
  if (terminal) gate[StreamingField.Status] |= StreamingStatus.Terminal;
  tryComplete();
}

/**
 * Call when the coordinator starts waiting on a not-yet-defined custom
 * element class before it can activate a boundary's deferred roots. Balanced
 * by exactly one `settleLateActivation()` when that wait resolves.
 */
export function markLateActivationPending(): void {
  requireStreamingGate()[StreamingField.LateActivations]++;
}

/**
 * Call when a previously pending late activation has resolved (the class was
 * defined and its deferred roots were activated, or the wait was abandoned).
 */
export function settleLateActivation(): void {
  const gate = streamingGate;
  if (!gate || gate[StreamingField.LateActivations] === 0) return;
  gate[StreamingField.LateActivations]--;
  tryComplete();
}

/**
 * Fire `webui:hydration-complete` once, when every known startup completion
 * condition is satisfied: no eager or active-batch hydration is running, and
 * on streaming pages the terminal boundary has committed with no boundary
 * still being processed. Dormant visibility-deferred roots are excluded.
 */
function tryComplete(): void {
  if (
    completed ||
    !documentReady ||
    pendingCount !== 0
  ) return;
  // The application can arrive before the independent coordinator asset.
  // The server's mode marker still reserves completion for its terminal.
  if (!streamingGate && hasBrowserDocument && isStreamingHydrationMode()) {
    requireStreamingGate();
  }
  const gate = streamingGate;
  if (!started && !gate) return;
  if (gate && (gate[StreamingField.Status] !== StreamingStatus.Terminal
    || gate[StreamingField.Boundaries] !== 0 || gate[StreamingField.LateActivations] !== 0)) return;

  completed = true;
  // A streaming page with no components at all reaches terminal without ever
  // calling hydrationStart(), so there is no start mark to measure against.
  if (started) {
    performance.mark('webui:hydrate:total:end');
    performance.measure(
      'webui:hydrate:total',
      'webui:hydrate:total:start',
      'webui:hydrate:total:end',
    );
  }
  window.dispatchEvent(new Event('webui:hydration-complete'));
}

// ── Test-only surface ─────────────────────────────────────────────────
// The lifecycle counters are module singletons shared with `streaming.ts`.
// Pipeline tests drive that shared instance, so they need a deterministic
// reset and read-only introspection to assert the streaming/late-activation
// accounting never underflows. Never referenced by production code.

/** Snapshot of the lifecycle counters for assertions. */
export function __getLifecycleStateForTests(): {
  pendingCount: number;
  started: boolean;
  completed: boolean;
  streamingGateActive: boolean;
  streamingGateAborted: boolean;
  terminalReached: boolean;
  pendingBoundaries: number;
  pendingLateActivations: number;
} {
  return {
    pendingCount,
    started,
    completed,
    streamingGateActive: streamingGate !== undefined,
    streamingGateAborted: !!((streamingGate?.[StreamingField.Status] ?? 0) & StreamingStatus.Aborted),
    terminalReached: !!((streamingGate?.[StreamingField.Status] ?? 0) & StreamingStatus.Terminal),
    pendingBoundaries: streamingGate?.[StreamingField.Boundaries] ?? 0,
    pendingLateActivations: streamingGate?.[StreamingField.LateActivations] ?? 0,
  };
}

/** Reset every lifecycle counter to its initial state. */
export function __resetLifecycleForTests(): void {
  pendingCount = 0;
  started = false;
  completed = false;
  streamingGate = undefined;
}
