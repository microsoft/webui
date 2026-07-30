// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Streaming priority-hydration entry point.
 *
 * The server streams `index.html` as priority-ordered `<boundary>`
 * chunks, using the Progressive Streaming Hydration contract from
 * DESIGN.md ("Progressive Streaming Hydration"):
 *
 * 1. The weather boundary commits first. It carries no server data, so it is
 *    the cheapest checkpoint on the page — `weather-panel` hydrates while the
 *    response is still open and immediately fetches its own forecast, which
 *    then overlaps the rest of the stream.
 * 2. The composer boundary (`message-composer`) commits next and must be
 *    interactive before `DOMContentLoaded`, while the response is still open.
 * 3. Three feed boundaries commit afterward, each with its own `feed-item`
 *    islands, hydrating independently and in order as their chunks arrive.
 *
 * The component modules import `@microsoft/webui-framework`, which no longer
 * installs the streaming coordinator on its own. This entry imports
 * `@microsoft/webui-framework/streaming.js` first — before those
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
// `weather-panel` is deliberately absent: it is an island whose `<script>`
// lives inside its own boundary, so its code is fetched when that chunk
// reaches the parser rather than being bundled into this critical entry.
import './message-composer/message-composer.js';
import './feed-item/feed-item.js';

// Fallback: if hydration already completed before the listener attached,
// log now instead of missing the event.
if (performance.getEntriesByName('webui:hydrate:total', 'measure').length > 0) {
  logHydrationTiming();
}
