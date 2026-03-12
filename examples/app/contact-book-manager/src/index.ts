/**
 * Contact-book-manager entry point — bootstraps FAST-HTML hydration
 * and the client-side router.
 */

performance.mark('contact-book-hydration-started');

import { TemplateElement } from '@microsoft/fast-html';
import { Router } from '@microsoft/webui-router';

// Shell component — eagerly loaded (child imports are co-located in each component)
import './cb-app/cb-app.js';

// Configure and start hydration
TemplateElement.options({
  'cb-app': { observerMap: 'all' },
  'cb-avatar': { observerMap: 'all' },
  'cb-badge': { observerMap: 'all' },
  'cb-button': { observerMap: 'all' },
  'cb-input': { observerMap: 'all' },
  'cb-icon-button': { observerMap: 'all' },
  'cb-empty-state': { observerMap: 'all' },
  'cb-search-bar': { observerMap: 'all' },
  'cb-form-field': { observerMap: 'all' },
  'cb-stat-card': { observerMap: 'all' },
  'cb-nav-item': { observerMap: 'all' },
  'cb-header': { observerMap: 'all' },
  'cb-sidebar': {},
}).config({
  hydrationComplete() {
    performance.measure('contact-book-hydration-completed', 'contact-book-hydration-started');
    console.log('Hydration complete!');
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
  },
}).define({
  name: 'f-template',
});
