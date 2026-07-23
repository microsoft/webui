// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Hydration lifecycle tracker.
 *
 * Tracks aggregate hydration timing via the Performance API and fires a
 * global `webui:hydration-complete` event on `window` once the initial
 * document is parsed and every registered component has finished hydrating.
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
 * `webui:hydration-complete` — dispatched once on `window` when the initial
 * document is parsed and no component is hydrating.
 */

const TOTAL_START_MARK = 'webui:hydrate:total:start';
const TOTAL_END_MARK = 'webui:hydrate:total:end';
const TOTAL_MEASURE = 'webui:hydrate:total';

/** How many components are actively hydrating. */
let pendingCount = 0;

/** Whether the global start mark has been placed. */
let started = false;

/** Whether the global complete event has already fired. */
let completed = false;

/** Whether initial document parsing and deferred/module scripts are complete. */
let documentReady = typeof document !== 'undefined' && document.readyState === 'complete';

/** Whether a completion check is already queued. */
let completionScheduled = false;

/** End time of the latest hydration that returned the active count to zero. */
let lastHydrationEndTime = 0;

function scheduleCompletion(): void {
  if (!documentReady || !started || pendingCount !== 0 || completed || completionScheduled) return;

  completionScheduled = true;
  queueMicrotask(() => {
    completionScheduled = false;
    if (!documentReady || pendingCount !== 0 || completed) return;

    completed = true;
    performance.mark(TOTAL_END_MARK, { startTime: lastHydrationEndTime });
    performance.measure(TOTAL_MEASURE, TOTAL_START_MARK, TOTAL_END_MARK);
    window.dispatchEvent(new Event('webui:hydration-complete'));
  });
}

function markDocumentReady(): void {
  if (documentReady) return;
  documentReady = true;
  scheduleCompletion();
}

if (typeof document !== 'undefined' && typeof window !== 'undefined' && !documentReady) {
  document.addEventListener('DOMContentLoaded', markDocumentReady, { once: true });
  window.addEventListener('load', markDocumentReady, { once: true });
}

/**
 * Call before a component begins hydration.
 * Increments the pending counter and (once) places the global start mark.
 */
export function hydrationStart(): void {
  if (!started) {
    performance.mark(TOTAL_START_MARK);
    started = true;
  }
  pendingCount++;
}

/**
 * Call after a component has finished hydration.
 * Records the latest possible aggregate end and completes once parsing is done.
 */
export function hydrationEnd(): void {
  pendingCount--;

  if (pendingCount !== 0 || completed) return;

  lastHydrationEndTime = performance.now();
  scheduleCompletion();
}
