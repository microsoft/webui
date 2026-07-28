// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Hydration lifecycle tracker.
 *
 * Tracks aggregate hydration timing via the Performance API and fires a
 * global `webui:hydration-complete` event on `window` once every registered
 * component has finished hydrating.
 *
 * ## Performance marks
 *
 * Global:
 * - `webui:hydrate:total:start`  — first component begins hydrating
 * - `webui:hydrate:total:end`    — last component finishes
 * - measure `webui:hydrate:total`
 *
 * ## Window event
 *
 * `webui:hydration-complete` — dispatched once on `window` when all
 * components are hydrated.
 *
 * ## Streaming gate
 *
 * A streaming-hydration page commits components across several boundaries
 * instead of all at once. `pendingCount` alone would let
 * `webui:hydration-complete` fire the instant an early boundary's components
 * finish, even though later boundaries (and the terminal record) haven't
 * arrived yet. The streaming coordinator (`streaming.ts`) opts a page into a
 * gate with `beginStreamingGate()`, then reports boundary lifecycle with
 * `markBoundaryPending()` / `markBoundaryCommitted()`. Non-streaming pages
 * never call these, so the gate stays inactive and behavior is unchanged.
 *
 * Streamed SSR hosts carry a compiler-owned `data-ws` identity, so the
 * completion gate does not need to expose parser-window state to components.
 */

/** How many components are still waiting to hydrate. */
let pendingCount = 0;

/** Whether the global start mark has been placed. */
let started = false;

/** Whether the global complete event has already fired. */
let completed = false;

/** Whether a streaming page has opted into the boundary-aware completion gate. */
let streamingGateActive = false;

/** Whether the terminal streaming record (`[version, seq, 1, {}]`) has committed. */
let terminalReached = false;

/**
 * Whether the streaming gate was aborted by the coordinator's failure path.
 * Once aborted, `webui:hydration-complete` must never fire: settling an
 * abandoned late-activation waiter during failure would otherwise drive the
 * pending counters to zero and dispatch completion for a stream that never
 * legitimately finished. A one-way latch — a failed stream cannot un-fail.
 */
let streamingGateAborted = false;

/** How many streamed boundaries have started committing but not finished. */
let pendingBoundaries = 0;

/**
 * How many deferred SSR roots are waiting on a not-yet-defined custom element
 * class to be activated by the streaming coordinator. A boundary can commit
 * (its scaffolding removed, markers gone) while some of its roots still can't
 * hydrate because their class hasn't loaded yet; completion must wait for
 * those late activations too, or `webui:hydration-complete` would fire while
 * roots are still inert. Accounted per unique undefined tag by the
 * coordinator, not per instance.
 */
let pendingLateActivations = 0;

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
  pendingCount--;
  tryComplete();
}

/** Opt this page into the streaming-aware completion gate. Idempotent. */
export function beginStreamingGate(): void {
  streamingGateActive = true;
}

/**
 * Abort the streaming gate: the coordinator hit an unrecoverable failure, so
 * `webui:hydration-complete` must never fire, even as failure cleanup settles
 * any outstanding boundary/late-activation counters. Idempotent and one-way.
 * A completion that already dispatched before the abort is unaffected (the
 * `completed` latch prevents a second dispatch either way).
 */
export function abortStreamingGate(): void {
  streamingGateAborted = true;
}

/** Call when a streamed boundary begins committing its roots. */
export function markBoundaryPending(): void {
  pendingBoundaries++;
}

/**
 * Call when a streamed boundary finishes committing its roots.
 * `terminal` is true for the boundary carrying the terminal record.
 */
export function markBoundaryCommitted(terminal: boolean): void {
  pendingBoundaries--;
  if (terminal) terminalReached = true;
  tryComplete();
}

/**
 * Call when the coordinator starts waiting on a not-yet-defined custom
 * element class before it can activate a boundary's deferred roots. Balanced
 * by exactly one `settleLateActivation()` when that wait resolves.
 */
export function markLateActivationPending(): void {
  pendingLateActivations++;
}

/**
 * Call when a previously pending late activation has resolved (the class was
 * defined and its deferred roots were activated, or the wait was abandoned).
 */
export function settleLateActivation(): void {
  pendingLateActivations--;
  tryComplete();
}

/**
 * Fire `webui:hydration-complete` once, when every known completion
 * condition is satisfied: no component is mid-hydration, and — only on
 * streaming pages — the terminal boundary has committed and no boundary is
 * still being processed.
 */
function tryComplete(): void {
  if (completed || streamingGateAborted || pendingCount > 0) return;
  if (streamingGateActive && (!terminalReached || pendingBoundaries > 0 || pendingLateActivations > 0)) return;

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
    streamingGateActive,
    streamingGateAborted,
    terminalReached,
    pendingBoundaries,
    pendingLateActivations,
  };
}

/** Reset every lifecycle counter to its initial state. */
export function __resetLifecycleForTests(): void {
  pendingCount = 0;
  started = false;
  completed = false;
  streamingGateActive = false;
  streamingGateAborted = false;
  terminalReached = false;
  pendingBoundaries = 0;
  pendingLateActivations = 0;
}
