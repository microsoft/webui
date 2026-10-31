// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { installTemplateRegistry, installTemplateResourcePreparation } from './template-registry.js';
import { installComponentStyleRuntime } from './element/style-runtime.js';
import { claimSsrComponentStyles, installMountedComponentStyles } from './element/styles.js';
import {
  cancelTemplateLinkStyleMount,
  COMPONENT_LINKS_EAGER,
  COMPONENT_ROOT_FRESH,
  installTemplateLinkStyles,
  preloadComponentAssetStyles,
  prepareComponentStyleLinks,
  prepareComponentStyleRoots,
  prepareRegisteredLinkStyles,
  prepareTemplateLinkStyles,
  templateMayContainAuthorStyles,
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
  // A shadow root attached in this mount holds nothing yet, so it can keep
  // its first Link stylesheet native and adopt shared sheets later. Roots
  // whose cascade has other author styles promote eagerly instead, so every
  // instance cascades the same way.
  install: (tag, target, host, freshShadow, meta) => installMountedComponentStyles(
    tag,
    target,
    host,
    !freshShadow
      ? COMPONENT_LINKS_EAGER
      : templateMayContainAuthorStyles(meta)
        ? COMPONENT_ROOT_FRESH | COMPONENT_LINKS_EAGER
        : COMPONENT_ROOT_FRESH,
  ),
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
