// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Contact-book-manager entry point — bootstraps WebUI Framework hydration
 * and the client-side router.
 */

import { Router } from '@microsoft/webui-router';

// Listen for the framework's global hydration-complete event.
window.addEventListener('webui:hydration-complete', onHydrationComplete);

function onHydrationComplete(): void {
  const total = performance.getEntriesByName('webui:hydrate:total', 'measure')[0];
  console.log(`Hydration complete in ${total?.duration.toFixed(1)}ms`);

  // Start router AFTER hydration — shadow roots are ready.
  // Page components use lazy loaders for code-split navigation.
  Router.start({
    loaders: {
      'cb-page-dashboard': () => import('./pages/cb-page-dashboard/cb-page-dashboard.js'),
      'cb-page-contacts': () => import('./pages/cb-page-contacts/cb-page-contacts.js'),
      'cb-page-favorites': () => import('./pages/cb-page-favorites/cb-page-favorites.js'),
      'cb-page-group': () => import('./pages/cb-page-group/cb-page-group.js'),
      'cb-contact-detail': () => import('./organisms/cb-contact-detail/cb-contact-detail.js'),
      'cb-contact-form': () => import('./organisms/cb-contact-form/cb-contact-form.js'),
    },
  });
}

// Shell component — eagerly loaded (child imports are co-located in each component)
import './cb-app/cb-app.js';

// Fallback: if hydration already completed before the listener, log now
if (performance.getEntriesByName('webui:hydrate:total', 'measure').length > 0) {
  onHydrationComplete();
}
