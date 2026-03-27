// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Todo-webui entry point — bootstraps WebUI Framework hydration.
 *
 * The server pre-renders HTML with hydration markers via `webui build --plugin=webui`.
 * Compiled templates are registered automatically by `<script>` blocks emitted
 * by the handler — no WTemplateElement needed.
 *
 * This script registers custom elements so they upgrade and hydrate.
 * The framework fires `webui:hydration-complete` on `window` once all
 * components have finished hydrating.
 */

// Listen for the framework's global hydration-complete event.
// NOTE: ES module imports are hoisted, so hydration may complete before
// this listener is registered. Check for the performance mark as a fallback.
window.addEventListener('webui:hydration-complete', logHydrationTiming);

function logHydrationTiming(): void {
  const total = performance.getEntriesByName('webui:hydrate:total', 'measure')[0];
  console.log(`Hydration complete in ${total?.duration.toFixed(1)}ms`);

  for (const entry of performance.getEntriesByType('measure')) {
    if (entry.name.startsWith('webui:hydrate:') && entry.name !== 'webui:hydrate:total') {
      console.log(`  ${entry.name}: ${entry.duration.toFixed(1)}ms`);
    }
  }
}

// Side-effect imports — register custom elements and trigger hydration
import './todo-app/todo-app.js';
import './todo-item/todo-item.js';

// Fallback: if hydration already completed before the listener, log now
if (performance.getEntriesByName('webui:hydrate:total', 'measure').length > 0) {
  logHydrationTiming();
}
