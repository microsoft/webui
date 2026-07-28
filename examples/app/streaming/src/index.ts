// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Streaming priority-hydration entry point.
 *
 * The server streams `index.html` as priority-ordered `<boundary>`
 * chunks, using the Progressive Streaming Hydration Phase 1 contract from
 * DESIGN.md ("Progressive Streaming Hydration — Phase 1"):
 *
 * 1. The composer boundary (`message-composer`) commits first and must be
 *    interactive before `DOMContentLoaded`, while the response is still open.
 * 2. The weather header renders a permanent skeleton (Phase 1 does not block
 *    the composer on it — see the comment in `index.html`).
 * 3. Three feed boundaries commit afterward, each with its own `feed-item`
 *    islands, hydrating independently and in order as their chunks arrive.
 *
 * `message-composer.js`/`feed-item.js` import `@microsoft/webui-framework`,
 * which no longer installs the streaming coordinator on its own. This entry
 * imports `@microsoft/webui-framework/streaming.js` first — before those
 * component side-effect imports — so the coordinator is installed and the
 * streaming gate is open before any authored `.define()` runs.
 */

// Install the streaming coordinator (side effect) before component imports so
// the gate is open before their `.define()` calls run.
import '@microsoft/webui-framework/streaming.js';

window.addEventListener('webui:hydration-complete', logHydrationTiming);

function logHydrationTiming(): void {
  const total = performance.getEntriesByName('webui:hydrate:total', 'measure')[0];
  if (total) {
    console.log(`Streaming hydration complete in ${total.duration.toFixed(1)}ms`);
  }
}

// Side-effect imports — register custom elements and trigger hydration.
import './message-composer/message-composer.js';
import './feed-item/feed-item.js';

// Fallback: if hydration already completed before the listener attached,
// log now instead of missing the event.
if (performance.getEntriesByName('webui:hydrate:total', 'measure').length > 0) {
  logHydrationTiming();
}
