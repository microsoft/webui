// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Compiler-owned TemplateElement host runtime.
 *
 * Scriptless templates are registered as custom elements so the router can
 * create them without empty authored modules. SSR instances remain dormant:
 * they do not walk DOM, consume bootstrap state, or install bindings until a
 * browser state write actually needs them. Client-created instances mount
 * immediately because they have no server-rendered DOM to preserve.
 */

import { TemplateElement } from './template-element.js';
import { getTemplateRegistry } from './template.js';
import { templateNeedsStaticHost } from './template-roots.js';
import {
  TEMPLATES_REGISTERED_EVENT,
  templateRegistrationDetail,
} from './template-events.js';
import type { TemplateMeta } from './template.js';

let runtimeInstalled = false;

/** Define the smallest client-rendering element for a compiler-owned template. */
function defineTemplateHost(tag: string, meta: TemplateMeta): void {
  const w = window as Window;
  if (!w.__webui) w.__webui = {};
  if (!w.__webui.templates) w.__webui.templates = {};
  if (!w.__webui.templates[tag]) w.__webui.templates[tag] = meta;

  class StaticTemplateHost extends TemplateElement {
    protected $afterExternalStateWrite(applied: boolean): void {
      if (applied) this.$activateDeferredSSR();
    }

    protected $shouldDeferSSRHydration(): boolean {
      return true;
    }

    protected $shouldApplySSRBootstrapState(): boolean {
      return false;
    }
  }

  StaticTemplateHost.define(tag);
}

/** Define a dormant host for one compiler-owned template tag when safe. */
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
 * Install the runtime for compiler-owned dormant template hosts.
 *
 * Called once by the framework root. Authored custom elements always win.
 */
export function installTemplateElementRuntime(): void {
  if (runtimeInstalled) {
    defineTemplateHosts();
    return;
  }
  if (typeof window === 'undefined' || typeof document === 'undefined') return;
  runtimeInstalled = true;

  window.addEventListener(TEMPLATES_REGISTERED_EVENT, (event: Event) => {
    const detail = templateRegistrationDetail(event);
    if (!detail?.templates) return;
    defineTemplateHosts(detail.templates);
  });

  if (document.readyState === 'loading') {
    document.addEventListener(
      'DOMContentLoaded',
      () => defineTemplateHosts(),
      { once: true },
    );
    return;
  }

  defineTemplateHosts();
}
