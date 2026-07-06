// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Static TemplateElement host runtime.
 *
 * The compiler marks exact HTML-only templates that need host attribute or
 * router state reactivity but have no authored `.ts` / `.js` implementation.
 * The framework root installs this module once; it defines missing static hosts
 * only for compiler-owned templates. Authored custom elements always win.
 */

import { TemplateElement } from './template-element.js';
import { getTemplateRegistry } from './template.js';
import { templateAttributeForRoot, templateNeedsStaticHost } from './template-roots.js';
import {
  TEMPLATES_REGISTERED_EVENT,
  templateRegistrationDetail,
} from './template-events.js';
import type { TemplateMeta } from './template.js';

let runtimeInstalled = false;
let initialClaimQueued = false;

/** Define the smallest hydrating element for a compiler-owned static template. */
function defineTemplateHost(tag: string, meta: TemplateMeta): void {
  const w = window as Window;
  if (!w.__webui) w.__webui = {};
  if (!w.__webui.templates) w.__webui.templates = {};
  if (!w.__webui.templates[tag]) w.__webui.templates[tag] = meta;

  class StaticTemplateHost extends TemplateElement {
    protected $shouldApplyTemplateStateFromSSR(key: string): boolean {
      const attr = templateAttributeForRoot(meta, key);
      return attr === undefined || !this.hasAttribute(attr);
    }
  }

  StaticTemplateHost.define(tag);
}

/**
 * Define a hydrating static host for one compiled template tag when safe.
 *
 * Developer-authored custom elements take precedence: when a tag is already
 * registered, this function leaves it untouched and reports no work.
 */
function defineMissingTemplateHost(tag: string, meta: TemplateMeta): void {
  if (!templateNeedsStaticHost(meta) || customElements.get(tag)) return;
  defineTemplateHost(tag, meta);
}

/** Claim every eligible template in a registry snapshot. */
function defineTemplateHosts(templates = getTemplateRegistry()): void {
  if (!templates) return;
  const tags = Object.keys(templates);
  for (let i = 0; i < tags.length; i++) {
    const tag = tags[i];
    const meta = templates[tag];
    if (meta) defineMissingTemplateHost(tag, meta);
  }
}

/**
 * Defer the first page-wide claim by one microtask.
 *
 * This gives authored component modules in the same import graph a chance to
 * call `customElements.define()` first, while router-delivered templates still
 * claim synchronously through the registration event below.
 */
function queueInitialStaticHostClaim(): void {
  if (initialClaimQueued) return;
  initialClaimQueued = true;
  queueMicrotask(() => {
    initialClaimQueued = false;
    defineTemplateHosts();
  });
}

/**
 * Install the runtime for compiler-owned static template hosts.
 *
 * Called once by the framework root. The compiler decides ownership per
 * component through `th`, so apps do not need a separate static-host bootstrap.
 */
export function installTemplateElementRuntime(): void {
  if (runtimeInstalled) {
    queueInitialStaticHostClaim();
    return;
  }
  if (typeof window === 'undefined' || typeof document === 'undefined') return;
  runtimeInstalled = true;

  window.addEventListener(TEMPLATES_REGISTERED_EVENT, (event: Event) => {
    const detail = templateRegistrationDetail(event);
    if (!detail) return;
    if (detail.templates) defineTemplateHosts(detail.templates);
  });

  if (document.readyState === 'loading') {
    document.addEventListener(
      'DOMContentLoaded',
      () => queueInitialStaticHostClaim(),
      { once: true },
    );
    return;
  }

  queueInitialStaticHostClaim();
}
