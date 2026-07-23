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
 */

/** How many components are still waiting to hydrate. */
let pendingCount = 0;

/** Whether the global start mark has been placed. */
let started = false;

/** Whether the global complete event has already fired. */
let completed = false;

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

  if (pendingCount <= 0 && !completed) {
    completed = true;
    performance.mark('webui:hydrate:total:end');
    performance.measure(
      'webui:hydrate:total',
      'webui:hydrate:total:start',
      'webui:hydrate:total:end',
    );
    window.dispatchEvent(new Event('webui:hydration-complete'));
  }
}
