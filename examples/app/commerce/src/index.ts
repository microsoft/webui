// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '@microsoft/webui-framework/runtime.js';
import '@microsoft/webui-framework/styles.js';
/**
 * WebUI Store — WebUI Framework hydration + client-side routing.
 *
 * The server pre-renders all HTML via WebUI's binary protocol (--plugin=webui).
 * This script registers interactive custom elements and activates the WebUI
 * Router. Scriptless components remain SSR-only and use document navigation.
 *
 * Navigation flow:
 *  1. Initial page load → full SSR + WebUI Framework hydration
 *  2. Subsequent navigations → Router intercepts via Navigation API,
 *     fetches JSON partial with state + templates, mounts page component
 *  3. Shell (mp-app: navbar, footer, cart) persists across navigations
 */
import { navigationCache, routePreload, routeBoundaries, viewTransitions } from '@microsoft/webui-delivery/router.js';
import { ndjson } from '@microsoft/webui-delivery/resources.js';
import { router } from './router.js';

// Shell and interactive children — eagerly loaded.
import '#organisms/mp-app/mp-app.js';
import '#organisms/mp-category-nav/mp-category-nav.js';
import '#organisms/mp-filter-list/mp-filter-list.js';
import '#organisms/mp-product-card/mp-product-card.js';

startRuntime();

// Listen for the framework's global hydration-complete event.
// NOTE: ES module imports are hoisted, so hydration may complete before
// this listener is registered. Check for the performance mark as a fallback.
window.addEventListener('webui:hydration-complete', onHydrationComplete);

function onHydrationComplete(): void {
  const total = performance.getEntriesByName('webui:hydrate:total', 'measure')[0];
  console.log(`WebUI Store hydration complete in ${total?.duration.toFixed(1)}ms`);

  // Start client-side router after hydration. Scriptless routes use document
  // navigation; the product page keeps its authored interactive class.
  router.start({
    transport: ndjson(),
    boundaries: routeBoundaries,
    transitions: viewTransitions,
    cache: navigationCache(),
    preload: routePreload(),
    loaders: {
      'mp-page-product': () => import('#pages/mp-page-product/mp-page-product.js'),
    },
  });
}

// Fallback: if hydration already completed before the listener, log now
if (performance.getEntriesByName('webui:hydrate:total', 'measure').length > 0) {
  onHydrationComplete();
}
