// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Calculator hydration entry point.
 *
 * The server pre-renders HTML with hydration markers via `webui build --plugin=webui`.
 * Registered custom elements hydrate through WebUI Framework. Importing the
 * framework through those components also installs static hosts for HTML-only
 * components without custom element stubs.
 */

window.addEventListener('webui:hydration-complete', logHydrationTiming);

function logHydrationTiming(): void {
  const total = performance.getEntriesByName('webui:hydrate:total', 'measure')[0];
  if (total) {
    console.log(`Calculator hydration complete in ${total.duration.toFixed(1)}ms`);
  }
}

// Side-effect imports — register custom elements and trigger hydration.
import './calc-app/calc-app.js';
import './calc-button/calc-button.js';

// Fallback: if hydration already completed before the listener, log now
if (performance.getEntriesByName('webui:hydrate:total', 'measure').length > 0) {
  logHydrationTiming();
}
