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

const enum HydrationPhase {
  Started = 1,
  Completed = 2,
  DocumentReady = 4,
  Installed = 8,
}

/** One document lifetime; ordinary pages allocate no separate lifecycle record. */
let phase = HydrationPhase.DocumentReady;

/** Install parser-readiness tracking explicitly, without side effects from API imports. */
export function installHydrationLifecycle(): void {
  if (phase & HydrationPhase.Installed) return;
  phase |= HydrationPhase.Installed;
  const hasBrowserDocument =
    typeof document !== 'undefined' &&
    typeof document.addEventListener === 'function';
  // The head marker precedes runtime startup, even when the independent
  // streaming coordinator arrives later. Reserve its completion gate once.
  if (hasBrowserDocument && isStreamingHydrationMode()) beginStreamingGate();
  const navigationTiming = hasBrowserDocument
    ? performance.getEntriesByType?.('navigation')[0] as PerformanceNavigationTiming | undefined
    : undefined;
  if (!hasBrowserDocument ||
    document.readyState === 'complete' ||
    (navigationTiming?.domContentLoadedEventStart ?? 0) > 0) return;
  phase &= ~HydrationPhase.DocumentReady;
  document.addEventListener('DOMContentLoaded', markDocumentReady, { once: true });
  // A late-loaded module can miss DOMContentLoaded when navigation timing is
  // unavailable. `load` is the next authoritative signal and cannot race
  // parser module startup.
  window.addEventListener('load', markDocumentReady, { once: true });
}

function markDocumentReady(): void {
  if (phase & HydrationPhase.DocumentReady) return;
  phase |= HydrationPhase.DocumentReady;
  tryComplete();
}

/** Whether parser/deferred module startup can still register hydration work. */
export function isHydrationStartupPending(): boolean {
  return !(phase & HydrationPhase.DocumentReady);
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

/**
 * Call before a component begins hydration.
 * Increments the pending counter and (once) places the global start mark.
 */
export function hydrationStart(): void {
  if (!(phase & HydrationPhase.Started)) {
    performance.mark('webui:hydrate:total:start');
    phase |= HydrationPhase.Started;
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

/** Reserve a synchronous definition cohort without starting its timing mark. */
export function beginHydrationBatch(): void {
  pendingCount++;
}

/** Opt this page into the streaming-aware completion gate. Idempotent. */
export function beginStreamingGate(): StreamingGate {
  return streamingGate ??= [0, 0, 0];
}

/**
 * Abort the streaming gate: the coordinator hit an unrecoverable failure, so
 * `webui:hydration-complete` must never fire, even as failure cleanup settles
 * any outstanding boundary/late-activation counters. Idempotent and one-way.
 * A completion that already dispatched before the abort is unaffected (the
 * `completed` latch prevents a second dispatch either way).
 */
export function abortStreamingGate(): void {
  beginStreamingGate()[StreamingField.Status] |= StreamingStatus.Aborted;
}

/** Call when a streamed boundary begins committing its roots. */
export function markBoundaryPending(): void {
  beginStreamingGate()[StreamingField.Boundaries]++;
}

/**
 * Call when a streamed boundary finishes committing its roots.
 * `terminal` is true for the boundary carrying the terminal record.
 */
export function markBoundaryCommitted(terminal: boolean): void {
  const gate = streamingGate;
  if (!gate || !gate[StreamingField.Boundaries]) return;
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
  beginStreamingGate()[StreamingField.LateActivations]++;
}

/**
 * Call when a previously pending late activation has resolved (the class was
 * defined and its deferred roots were activated, or the wait was abandoned).
 */
export function settleLateActivation(): void {
  const gate = streamingGate;
  if (!gate || !gate[StreamingField.LateActivations]) return;
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
    (phase & (HydrationPhase.Completed | HydrationPhase.DocumentReady)) !== HydrationPhase.DocumentReady ||
    pendingCount !== 0
  ) return;
  const gate = streamingGate;
  if (gate
    ? gate[StreamingField.Status] !== StreamingStatus.Terminal
      || gate[StreamingField.Boundaries] || gate[StreamingField.LateActivations]
    : !(phase & HydrationPhase.Started)) return;

  phase |= HydrationPhase.Completed;
  // A streaming page with no components at all reaches terminal without ever
  // calling hydrationStart(), so there is no start mark to measure against.
  if (phase & HydrationPhase.Started) {
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
    started: !!(phase & HydrationPhase.Started),
    completed: !!(phase & HydrationPhase.Completed),
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
  phase &= ~(HydrationPhase.Started | HydrationPhase.Completed);
  streamingGate = undefined;
}
