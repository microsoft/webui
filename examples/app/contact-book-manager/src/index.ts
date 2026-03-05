/**
 * Contact-book-manager entry point — bootstraps FAST-HTML hydration.
 *
 * The server pre-renders HTML with hydration markers via `webui build --plugin=fast`.
 * This script:
 *   1. Registers custom elements (17 components) via defineAsync
 *   2. Configures FAST-HTML observation maps for reactive attribute tracking
 *   3. Defines <f-template>, triggering hydration of pre-rendered shadow DOM
 */

performance.mark('contact-book-hydration-started');

import { TemplateElement } from '@microsoft/fast-html';

// Side-effect imports — register custom elements via defineAsync

// Root
import './cb-app/cb-app.js';

// Atoms
import './atoms/cb-avatar/cb-avatar.js';
import './atoms/cb-badge/cb-badge.js';
import './atoms/cb-button/cb-button.js';
import './atoms/cb-input/cb-input.js';
import './atoms/cb-icon-button/cb-icon-button.js';
import './atoms/cb-empty-state/cb-empty-state.js';

// Molecules
import './molecules/cb-search-bar/cb-search-bar.js';
import './molecules/cb-form-field/cb-form-field.js';
import './molecules/cb-stat-card/cb-stat-card.js';
import './molecules/cb-nav-item/cb-nav-item.js';

// Organisms
import './organisms/cb-header/cb-header.js';
import './organisms/cb-sidebar/cb-sidebar.js';
import './organisms/cb-contact-card/cb-contact-card.js';
import './organisms/cb-contact-list/cb-contact-list.js';
import './organisms/cb-contact-detail/cb-contact-detail.js';
import './organisms/cb-contact-form/cb-contact-form.js';

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
  'cb-contact-card': { observerMap: 'all' },
  'cb-contact-list': { observerMap: 'all' },
  'cb-contact-detail': { observerMap: 'all' },
  'cb-contact-form': { observerMap: 'all' },
}).config({
  hydrationComplete() {
    performance.measure('contact-book-hydration-completed', 'contact-book-hydration-started');
    console.log('Hydration complete!');
  },
}).define({
  name: 'f-template',
});
