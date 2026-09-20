// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Contact-book-manager entry point — bootstraps WebUI Framework hydration
 * and the client-side router.
 */

import { Router } from '@microsoft/webui-router';

// Shell component — eagerly loaded.
import './cb-app/cb-app.js';

// Listen for the framework's global hydration-complete event.
window.addEventListener('webui:hydration-complete', onHydrationComplete);

function onHydrationComplete(): void {
  const total = performance.getEntriesByName('webui:hydrate:total', 'measure')[0];
  console.log(`Hydration complete in ${total?.duration.toFixed(1)}ms`);

  // Start router AFTER hydration — shadow roots are ready.
  // Page components use lazy loaders for code-split navigation.
  Router.start({
    loaders: {
      'cb-contact-detail': () => import('./organisms/cb-contact-detail/cb-contact-detail.js'),
      'cb-contact-form': () => import('./organisms/cb-contact-form/cb-contact-form.js'),
    },
  });
}

// Fallback: if hydration already completed before the listener, log now
if (performance.getEntriesByName('webui:hydrate:total', 'measure').length > 0) {
  onHydrationComplete();
}
