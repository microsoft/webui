// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * WebUI Store — WebUI Framework hydration + client-side routing.
 *
 * The server pre-renders all HTML via WebUI's binary protocol (--plugin=webui).
 * This script registers interactive custom elements, triggers hydration,
 * and activates the WebUI Router for SPA page transitions.
 *
 * Navigation flow:
 *  1. Initial page load → full SSR + WebUI Framework hydration
 *  2. Subsequent navigations → Router intercepts via Navigation API,
 *     fetches JSON partial with state + templates, mounts page component
 *  3. Shell (mp-app: navbar, footer, cart) persists across navigations
 */

import { Router } from '@microsoft/webui-router';

// Listen for the framework's global hydration-complete event.
// NOTE: ES module imports are hoisted, so hydration may complete before
// this listener is registered. Check for the performance mark as a fallback.
window.addEventListener('webui:hydration-complete', onHydrationComplete);

function onHydrationComplete(): void {
  const total = performance.getEntriesByName('webui:hydrate:total', 'measure')[0];
  console.log(`WebUI Store hydration complete in ${total?.duration.toFixed(1)}ms`);

  // Start client-side router after hydration — page components lazy-loaded
  Router.start({
    loaders: {
      'mp-page-home': () => import('#pages/mp-page-home/mp-page-home.js'),
      'mp-page-search': () => import('#pages/mp-page-search/mp-page-search.js'),
      'mp-product-grid': () => import('#organisms/mp-product-grid/mp-product-grid.js'),
      'mp-page-product': () => import('#pages/mp-page-product/mp-page-product.js'),
      'mp-page-about': () => import('#pages/mp-page-about/mp-page-about.js'),
      'mp-page-terms': () => import('#pages/mp-page-terms/mp-page-terms.js'),
      'mp-page-shipping': () => import('#pages/mp-page-shipping/mp-page-shipping.js'),
      'mp-page-privacy': () => import('#pages/mp-page-privacy/mp-page-privacy.js'),
      'mp-page-faq': () => import('#pages/mp-page-faq/mp-page-faq.js'),
    },
  });
}

// Shell component — eagerly loaded (child imports are co-located in each component)
import '#organisms/mp-app/mp-app.js';

// Search page components — eagerly loaded for SSR hydration of nested routes.
import '#pages/mp-page-search/mp-page-search.js';
import '#organisms/mp-product-grid/mp-product-grid.js';

// Fallback: if hydration already completed before the listener, log now
if (performance.getEntriesByName('webui:hydrate:total', 'measure').length > 0) {
  onHydrationComplete();
}
