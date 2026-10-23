// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { templateHtmlMayContainLink } from '../template-content.js';
import type { TemplateMeta } from '../template-types.js';
import { hasComponentStyleWork } from './style-catalog.js';

export interface ComponentStyleRuntime {
  readonly claimSsr: typeof import('./styles.js').claimSsrComponentStyles;
  readonly install: typeof import('./styles.js').installComponentStyles;
  readonly prepareTemplate: typeof import('./link-styles.js').prepareTemplateLinkStyles;
  readonly hasTemplateLinks: typeof import('./link-styles.js').templateMayContainLinkStyles;
  readonly installTemplateLinks: typeof import('./link-styles.js').installTemplateLinkStyles;
  readonly cancelTemplateMount: typeof import('./link-styles.js').cancelTemplateLinkStyleMount;
}

export let componentStyleRuntime: ComponentStyleRuntime | undefined;

const templatesWithoutLinks = new WeakSet<TemplateMeta>();

export function installComponentStyleRuntime(runtime: ComponentStyleRuntime): void {
  if (componentStyleRuntime && componentStyleRuntime !== runtime) {
    throw new Error('[WebUI] A component style runtime is already installed. Import one shared framework instance.');
  }
  componentStyleRuntime = runtime;
}

export function requireComponentStyleRuntime(
  tag: string,
  meta: TemplateMeta,
  ownerDocument: Document = globalThis.document,
): ComponentStyleRuntime | undefined {
  if (componentStyleRuntime) return componentStyleRuntime;
  if (hasComponentStyleWork(tag, ownerDocument) || containsTemplateLinks(meta)) {
    throw new Error(
      `[WebUI] Styles for <${tag}> require @microsoft/webui-framework/styles.js. ` +
      'Import that entry before startRuntime().',
    );
  }
  return undefined;
}

function containsTemplateLinks(meta: TemplateMeta): boolean {
  if (templatesWithoutLinks.has(meta)) return false;
  if (templateHtmlMayContainLink(meta.h)) return true;
  const blocks = meta.b;
  if (blocks) {
    for (let i = 0; i < blocks.length; i++) {
      if (templateHtmlMayContainLink(blocks[i].h)) return true;
    }
  }
  templatesWithoutLinks.add(meta);
  return false;
}
