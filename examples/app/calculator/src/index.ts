// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Calculator hydration entry point.
 *
 * The server pre-renders HTML with hydration markers via `webui build --plugin=webui`.
 * Registered custom elements hydrate through WebUI Framework. The entrypoint
 * opts into the HTML-only runtime for the display component and listens for
 * `webui:hydration-complete` on `window` once they finish.
 */

import { installAutoElementRuntime } from '@microsoft/webui-framework/auto-element.js';

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

installAutoElementRuntime();

// Fallback: if hydration already completed before the listener, log now
if (performance.getEntriesByName('webui:hydrate:total', 'measure').length > 0) {
  logHydrationTiming();
}
