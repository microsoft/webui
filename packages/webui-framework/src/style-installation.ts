// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { installTemplateResourcePreparation } from './template-registry.js';
import { installComponentStyleRuntime, type ComponentStyleRuntime } from './element/style-runtime.js';
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

const runtime: ComponentStyleRuntime = {
  claimSsr: claimSsrComponentStyles,
  // Fresh roots keep their first native link; roots with other author styles
  // promote eagerly so every instance keeps the same cascade.
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
};

function registerStyles(value: unknown): Promise<void> | undefined {
  const styles = requireComponentStyles(value);
  validateComponentStylesRegistration(styles);
  registerPreparedComponentStyles(styles);
  return prepareComponentStyleLinks(styles);
}

export function installStylesRuntime(): void {
  installComponentStyleRuntime(runtime);
  if (typeof window !== 'undefined' && typeof window.addEventListener === 'function') {
    window.__webuiPrepareResources = prepareComponentStyleRoots;
    window.__webuiPreloadStyles = preloadComponentAssetStyles;
    installTemplateResourcePreparation(prepareRegisteredLinkStyles);
    window.__webuiRegisterComponentStyles = registerStyles;
  }
}
