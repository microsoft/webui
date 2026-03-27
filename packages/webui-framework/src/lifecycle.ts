// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Hydration lifecycle tracker.
 *
 * Tracks per-component hydration timing via the Performance API and fires a
 * global `webui:hydration-complete` event on `window` once every registered
 * component has finished hydrating.
 *
 * ## Performance marks
 *
 * Per component instance:
 * - `webui:hydrate:<tag>:start`
 * - `webui:hydrate:<tag>:end`
 * - measure `webui:hydrate:<tag>` between the two
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
 * Places per-component and (once) global start marks.
 */
export function hydrationStart(tagName: string): void {
  if (!started) {
    performance.mark('webui:hydrate:total:start');
    started = true;
  }
  pendingCount++;
  performance.mark(`webui:hydrate:${tagName}:start`);
}

/**
 * Call after a component has finished hydration.
 * Places per-component marks/measures and, when the last component
 * finishes, fires the global event + measure.
 */
export function hydrationEnd(tagName: string): void {
  performance.mark(`webui:hydrate:${tagName}:end`);
  performance.measure(
    `webui:hydrate:${tagName}`,
    `webui:hydrate:${tagName}:start`,
    `webui:hydrate:${tagName}:end`,
  );

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
