// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { CoreElement } from './element.js';
import { toKebabCase } from './decorators.js';
import { getTemplateRegistry } from './template.js';
import { templateHasEventHandlers } from './template-roots.js';
import {
  TEMPLATES_REGISTERED_EVENT,
  templateRegistrationDetail,
} from './template-events.js';
import type { TemplateMeta } from './template.js';

let runtimeInstalled = false;
let initialClaimQueued = false;

function defineAutoElement(tag: string, meta: TemplateMeta): void {
  const w = window as Window;
  if (!w.__webui) w.__webui = {};
  if (!w.__webui.templates) w.__webui.templates = {};
  if (!w.__webui.templates[tag]) w.__webui.templates[tag] = meta;

  class AutoWebUIElement extends CoreElement {
    protected $shouldApplyTemplateStateFromSSR(key: string): boolean {
      return !this.hasAttribute(toKebabCase(key));
    }
  }

  AutoWebUIElement.define(tag);
}

/**
 * Define a hydrating fallback element for one compiled template tag when safe.
 *
 * Developer-authored custom elements take precedence: when a tag is already
 * registered, this function leaves it untouched and reports no fallback work.
 */
function defineMissingTemplateElement(tag: string, meta: TemplateMeta): boolean {
  if (
    typeof customElements === 'undefined' ||
    typeof HTMLElement === 'undefined' ||
    !meta.ae ||
    customElements.get(tag) ||
    templateHasEventHandlers(meta)
  ) {
    return false;
  }
  defineAutoElement(tag, meta);
  return true;
}

function defineAutoTemplateElements(templates = getTemplateRegistry()): void {
  if (!templates) return;
  const tags = Object.keys(templates);
  for (let i = 0; i < tags.length; i++) {
    const tag = tags[i];
    const meta = templates[tag];
    if (meta) defineMissingTemplateElement(tag, meta);
  }
}

function queueInitialAutoElementClaim(): void {
  if (initialClaimQueued) return;
  initialClaimQueued = true;
  queueMicrotask(() => {
    initialClaimQueued = false;
    defineAutoTemplateElements();
  });
}

/**
 * Install the fallback runtime for compiler-marked HTML-only compiled templates.
 */
export function installAutoElementRuntime(): void {
  if (runtimeInstalled) {
    queueInitialAutoElementClaim();
    return;
  }
  if (typeof window === 'undefined' || typeof document === 'undefined') return;
  runtimeInstalled = true;

  window.addEventListener(TEMPLATES_REGISTERED_EVENT, (event: Event) => {
    const templates = templateRegistrationDetail(event);
    if (templates) defineAutoTemplateElements(templates);
  });

  if (document.readyState === 'loading') {
    document.addEventListener(
      'DOMContentLoaded',
      () => defineAutoTemplateElements(),
      { once: true },
    );
    return;
  }

  queueInitialAutoElementClaim();
}
