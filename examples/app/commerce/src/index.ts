// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * WebUI Store — FAST-HTML hydration + client-side routing.
 *
 * The server pre-renders all HTML via WebUI's binary protocol (--plugin=fast).
 * This script registers interactive custom elements, triggers hydration,
 * and activates the WebUI Router for SPA page transitions.
 *
 * Navigation flow:
 *  1. Initial page load → full SSR + FAST-HTML hydration
 *  2. Subsequent navigations → Router intercepts via Navigation API,
 *     fetches JSON partial with state + templates, mounts page component
 *  3. Shell (mp-app: navbar, footer, cart) persists across navigations
 */

performance.mark('marketplace-hydration-started');

import { TemplateElement } from '@microsoft/fast-html';
import { Router } from '@microsoft/webui-router';

// Shell component — eagerly loaded (child imports are co-located in each component)
import '#organisms/mp-app/mp-app.js';

// Configure and start hydration
TemplateElement.options({
  'mp-app': { observerMap: 'all' },
  'mp-navbar': { observerMap: 'all' },
  'mp-mobile-menu': { observerMap: 'all' },
  'mp-category-nav': { observerMap: 'all' },
  'mp-cart-panel': { observerMap: 'all' },
  'mp-footer': { observerMap: 'all' },
  'mp-add-to-cart': { observerMap: 'all' },
  'mp-carousel': { observerMap: 'all' },
  'mp-filter-list': { observerMap: 'all' },
  'mp-hero-grid': { observerMap: 'all' },
  'mp-product-card': { observerMap: 'all' },
  'mp-product-gallery': { observerMap: 'all' },
  'mp-product-grid': { observerMap: 'all' },
  'mp-variant-selector': { observerMap: 'all' },
  'mp-page-home': { observerMap: 'all' },
  'mp-page-search': { observerMap: 'all' },
  'mp-page-product': { observerMap: 'all' },
  'mp-page-about': { observerMap: 'all' },
  'mp-page-terms': { observerMap: 'all' },
  'mp-page-shipping': { observerMap: 'all' },
  'mp-page-privacy': { observerMap: 'all' },
  'mp-page-faq': { observerMap: 'all' },
  'mp-icon': { observerMap: 'all' },
  'mp-price': { observerMap: 'all' },
  'mp-product-image': { observerMap: 'all' },
  'mp-product-label': { observerMap: 'all' },
  'mp-search-bar': { observerMap: 'all' },
}).config({
  hydrationComplete() {
    performance.measure('marketplace-hydration-completed', 'marketplace-hydration-started');
    console.log('WebUI Store hydration complete!');

    // Start client-side router after hydration — page components lazy-loaded
    Router.start({
      loaders: {
        'mp-page-home': () => import('#pages/mp-page-home/mp-page-home.js'),
        'mp-page-search': () => import('#pages/mp-page-search/mp-page-search.js'),
        'mp-page-product': () => import('#pages/mp-page-product/mp-page-product.js'),
        'mp-page-about': () => import('#pages/mp-page-about/mp-page-about.js'),
        'mp-page-terms': () => import('#pages/mp-page-terms/mp-page-terms.js'),
        'mp-page-shipping': () => import('#pages/mp-page-shipping/mp-page-shipping.js'),
        'mp-page-privacy': () => import('#pages/mp-page-privacy/mp-page-privacy.js'),
        'mp-page-faq': () => import('#pages/mp-page-faq/mp-page-faq.js'),
      },
    });
  },
}).define({
  name: 'f-template',
});
