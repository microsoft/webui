// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Routes example — nested routing with WebUI Framework.
 *
 * Route structure:
 *   / → routes-app (shell with nav)
 *     ./sections/:id → section-page (topics list)
 *       ./topics/:topicId → topic-page (lessons list)
 *         ./lessons/:lessonId → lesson-page (lesson content)
 */

import { Router } from '@microsoft/webui-router';
import { installAutoElementRuntime } from '@microsoft/webui-framework/auto-element.js';

// Listen for the framework's global hydration-complete event.
// NOTE: ES module imports are hoisted, so hydration may complete before
// this listener is registered. Check for the performance mark as a fallback.
window.addEventListener('webui:hydration-complete', onHydrationComplete);

function onHydrationComplete(): void {
  const total = performance.getEntriesByName('webui:hydrate:total', 'measure')[0];
  console.log(`Hydration complete in ${total?.duration.toFixed(1)}ms`);

  Router.start({
    loaders: {
      'section-page': () => import('./section-page/section-page.js'),
      'topic-page': () => import('./topic-page/topic-page.js'),
    },
  });
}

// Side-effect imports — register custom elements and trigger hydration
import './routes-app/routes-app.js';

installAutoElementRuntime();

// Fallback: if hydration already completed before the listener, log now
if (performance.getEntriesByName('webui:hydrate:total', 'measure').length > 0) {
  onHydrationComplete();
}
