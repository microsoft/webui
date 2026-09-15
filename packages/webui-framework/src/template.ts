// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

export * from './template-registry.js';

import { installTemplateResourcePreparation } from './template-registry.js';
import {
  registerPreparedComponentStyles,
  requireComponentStyles,
  validateComponentStylesRegistration,
} from './element/style-catalog.js';
import {
  prepareComponentStyleLinks,
  prepareRegisteredLinkStyles,
} from './element/link-styles.js';

// Streaming registers the same catalogs without importing client-mount CSS
// preparation. The hydration runtime installs that work only when it loads.
if (typeof window !== 'undefined' && typeof window.addEventListener === 'function') {
  installTemplateResourcePreparation(prepareRegisteredLinkStyles);
  window.__webuiRegisterComponentStyles = (value: unknown): Promise<void> | undefined => {
    const styles = requireComponentStyles(value);
    validateComponentStylesRegistration(styles);
    registerPreparedComponentStyles(styles);
    return prepareComponentStyleLinks(styles);
  };
}
