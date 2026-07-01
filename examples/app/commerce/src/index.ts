// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * WebUI Store — WebUI Framework hydration + client-side routing.
 *
 * The server pre-renders all HTML via WebUI's binary protocol (--plugin=webui).
 * This script registers interactive custom elements, lets the framework
 * auto-define HTML-only template elements, and activates the WebUI Router for
 * SPA page transitions.
 *
 * Navigation flow:
 *  1. Initial page load → full SSR + WebUI Framework hydration
 *  2. Subsequent navigations → Router intercepts via Navigation API,
 *     fetches JSON partial with state + templates, mounts page component
 *  3. Shell (mp-app: navbar, footer, cart) persists across navigations
 */

import { Router } from '@microsoft/webui-router';

// Shell and interactive children — eagerly loaded. HTML-only tags are
// auto-claimed from compiled template metadata.
import '#organisms/mp-app/mp-app.js';
import '#organisms/mp-category-nav/mp-category-nav.js';
import '#organisms/mp-filter-list/mp-filter-list.js';
import '#organisms/mp-product-card/mp-product-card.js';

// Listen for the framework's global hydration-complete event.
// NOTE: ES module imports are hoisted, so hydration may complete before
// this listener is registered. Check for the performance mark as a fallback.
window.addEventListener('webui:hydration-complete', onHydrationComplete);

function onHydrationComplete(): void {
  const total = performance.getEntriesByName('webui:hydrate:total', 'measure')[0];
  console.log(`WebUI Store hydration complete in ${total?.duration.toFixed(1)}ms`);

  // Start client-side router after hydration. HTML-only routes use template
  // fallback elements; the product page keeps its authored interactive class.
  Router.start({
    preload: true,
    loaders: {
      'mp-page-product': () => import('#pages/mp-page-product/mp-page-product.js'),
    },
  });
}

// Fallback: if hydration already completed before the listener, log now
if (performance.getEntriesByName('webui:hydrate:total', 'measure').length > 0) {
  onHydrationComplete();
}
