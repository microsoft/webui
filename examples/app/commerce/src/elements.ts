// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Eagerly loaded custom element definitions.
 *
 * All elements are imported in a single module so that every
 * customElements.define() call runs before any element upgrades.
 * This avoids staggered upgrade batches that each trigger a separate
 * style recalc + layout pass.
 */

// Atoms
import '#atoms/mp-price/mp-price.js';
import '#atoms/mp-product-image/mp-product-image.js';

// Molecules
import '#molecules/mp-product-label/mp-product-label.js';
import '#molecules/mp-search-bar/mp-search-bar.js';

// Organisms — shell
import '#organisms/mp-navbar/mp-navbar.js';
import '#organisms/mp-mobile-menu/mp-mobile-menu.js';
import '#organisms/mp-cart-panel/mp-cart-panel.js';
import '#organisms/mp-footer/mp-footer.js';
import '#organisms/mp-app/mp-app.js';

// Organisms — content
import '#organisms/mp-category-nav/mp-category-nav.js';
import '#organisms/mp-filter-list/mp-filter-list.js';
import '#organisms/mp-product-card/mp-product-card.js';
import '#organisms/mp-product-grid/mp-product-grid.js';

// Pages — eagerly loaded for SSR hydration
import '#pages/mp-page-search/mp-page-search.js';
