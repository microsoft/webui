// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { installTemplateRegistry, installTemplateResourcePreparation } from './template-registry.js';
import { installComponentStyleRuntime } from './element/style-runtime.js';
import { claimSsrComponentStyles, installComponentStyles } from './element/styles.js';
import {
  cancelTemplateLinkStyleMount,
  installTemplateLinkStyles,
  preloadComponentAssetStyles,
  prepareComponentStyleLinks,
  prepareComponentStyleRoots,
  prepareRegisteredLinkStyles,
  prepareTemplateLinkStyles,
  templateMayContainLinkStyles,
} from './element/link-styles.js';
import {
  registerPreparedComponentStyles,
  requireComponentStyles,
  validateComponentStylesRegistration,
} from './element/style-catalog.js';

installTemplateRegistry();
installComponentStyleRuntime({
  claimSsr: claimSsrComponentStyles,
  install: installComponentStyles,
  prepareTemplate: prepareTemplateLinkStyles,
  hasTemplateLinks: templateMayContainLinkStyles,
  installTemplateLinks: installTemplateLinkStyles,
  cancelTemplateMount: cancelTemplateLinkStyleMount,
});

if (typeof window !== 'undefined' && typeof window.addEventListener === 'function') {
  window.__webuiPrepareResources = prepareComponentStyleRoots;
  window.__webuiPreloadStyles = preloadComponentAssetStyles;
  installTemplateResourcePreparation(prepareRegisteredLinkStyles);
  window.__webuiRegisterComponentStyles = (value: unknown): Promise<void> | undefined => {
    const styles = requireComponentStyles(value);
    validateComponentStylesRegistration(styles);
    registerPreparedComponentStyles(styles);
    return prepareComponentStyleLinks(styles);
  };
}

export {
  installComponentStyles,
  prepareComponentStyles,
  registerComponentStyles,
} from './element/styles.js';
export type {
  ComponentStyleResource,
  ComponentStyles,
} from './element/style-catalog.js';
