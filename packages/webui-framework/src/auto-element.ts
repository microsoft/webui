// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Automatic HTML-only component runtime.
 *
 * The parser marks component templates that have no sibling `.ts` / `.js`
 * implementation. Importing the framework root installs this runtime so those
 * scriptless templates still hydrate as real WebUI elements when server or route
 * state changes. Authored custom elements always win, and templates with event
 * metadata are refused because event handlers require developer code.
 *
 * Keep this module dependent on `CoreElement`, not `WebUIElement`: the whole
 * point is that HTML-only pages can tree-shake event, ref, and `$emit` support.
 */

import { CoreElement } from './element.js';
import { toKebabCase } from './decorators.js';
import { getTemplateRegistry } from './template.js';
import { templateNeedsAutoElement } from './template-roots.js';
import {
  TEMPLATES_REGISTERED_EVENT,
  templateRegistrationDetail,
} from './template-events.js';
import type { TemplateMeta } from './template.js';

let runtimeInstalled = false;
let initialClaimQueued = false;
const blockedAutoElementTags = new Set<string>();

/** Define the smallest hydrating element for a scriptless template. */
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
 * Define a hydrating auto-element for one compiled template tag when safe.
 *
 * Developer-authored custom elements take precedence: when a tag is already
 * registered, this function leaves it untouched and reports no work.
 */
function defineMissingTemplateElement(tag: string, meta: TemplateMeta): void {
  if (
    !templateNeedsAutoElement(meta) ||
    blockedAutoElementTags.has(tag) ||
    customElements.get(tag)
  ) {
    return;
  }
  defineAutoElement(tag, meta);
}

/** Claim every eligible template in a registry snapshot. */
function defineAutoTemplateElements(templates = getTemplateRegistry()): void {
  if (!templates) return;
  const tags = Object.keys(templates);
  for (let i = 0; i < tags.length; i++) {
    const tag = tags[i];
    const meta = templates[tag];
    if (meta) defineMissingTemplateElement(tag, meta);
  }
}

/**
 * Defer the first page-wide claim by one microtask.
 *
 * This gives authored component modules in the same import graph a chance to
 * call `customElements.define()` first, while router-delivered templates still
 * claim synchronously through the registration event below.
 */
function queueInitialAutoElementClaim(): void {
  if (initialClaimQueued) return;
  initialClaimQueued = true;
  queueMicrotask(() => {
    initialClaimQueued = false;
    defineAutoTemplateElements();
  });
}

/**
 * Install the runtime for compiler-marked HTML-only templates.
 *
 * This is called by the package root as a side effect, so app authors do not
 * maintain tag lists or import an auto-element subpath.
 */
export function installAutoElementRuntime(): void {
  if (runtimeInstalled) {
    queueInitialAutoElementClaim();
    return;
  }
  if (typeof window === 'undefined' || typeof document === 'undefined') return;
  runtimeInstalled = true;

  window.addEventListener(TEMPLATES_REGISTERED_EVENT, (event: Event) => {
    const detail = templateRegistrationDetail(event);
    if (!detail) return;
    const blockedTags = detail.blockedTags;
    if (blockedTags) {
      for (let i = 0; i < blockedTags.length; i++) {
        blockedAutoElementTags.add(blockedTags[i]);
      }
    }
    if (detail.templates) defineAutoTemplateElements(detail.templates);
  });

  if (document.readyState === 'loading') {
    document.addEventListener(
      'DOMContentLoaded',
      () => queueInitialAutoElementClaim(),
      { once: true },
    );
    return;
  }

  queueInitialAutoElementClaim();
}
