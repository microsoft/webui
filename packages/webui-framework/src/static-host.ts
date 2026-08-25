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

import {
  consumePendingParentState,
  TemplateElement,
} from './template-element.js';
import { hasComponentStyleWork } from './element/styles.js';
import { getTemplateRegistry } from './template.js';
import { templateNeedsStaticHost } from './template-roots.js';
import {
  ACTIVATION_STATIC_HOST_OPT_OUT,
  isStreamingHydrationMode,
  STREAMED_HOST_ATTR,
  STREAMING_BOUNDARY_ACTIVATE,
} from './streaming-mode.js';
import {
  TEMPLATES_REGISTERED_EVENT,
  templateRegistrationDetail,
} from './template-events.js';
import type { TemplateMeta } from './template.js';

let runtimeInstalled = false;

// Static hosts are defined in a later task, so parent `:` writes can arrive
// first. The WeakMap keeps hosts with no queued writes field-free.
function applyPendingNoopHostState(host: HTMLElement): void {
  const pending = consumePendingParentState(host);
  if (!pending) return;
  const target = host as unknown as Record<string, unknown>;
  const keys = Object.keys(pending.values);
  for (let i = 0; i < keys.length; i++) {
    const key = keys[i];
    if (!Object.hasOwn(target, key)) target[key] = pending.values[key];
  }
}

/** Define the smallest client-rendering element for a compiler-owned template. */
function defineTemplateHost(tag: string, meta: TemplateMeta): void {
  const w = window as Window;
  if (!w.__webui) w.__webui = {};
  if (!w.__webui.templates) w.__webui.templates = {};
  if (!w.__webui.templates[tag]) w.__webui.templates[tag] = meta;

  if (templateIsNoopHost(meta) && !hasComponentStyleWork(tag)) {
    // Prototype-only behavior avoids TemplateElement's per-instance state.
    customElements.define(tag, class extends HTMLElement {
      connectedCallback(): void {
        // Preserve streamed deferral before a queued property can shadow
        // hasAttribute(). Ordinary data-ws attributes remain author-owned.
        if (
          isStreamingHydrationMode() &&
          this.hasAttribute(STREAMED_HOST_ATTR)
        ) {
          return;
        }
        applyPendingNoopHostState(this);
      }

      [STREAMING_BOUNDARY_ACTIVATE](): typeof ACTIVATION_STATIC_HOST_OPT_OUT {
        applyPendingNoopHostState(this);
        return ACTIVATION_STATIC_HOST_OPT_OUT;
      }
    });
    return;
  }

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

    // Streaming boundaries have no per-root activation signal, so
    // a committed boundary alone must not wake a compiler-owned host — it
    // stays dormant until an explicit client state write, same as today.
    protected $shouldActivateOnBoundaryCommit(): boolean {
      return false;
    }
  }

  StaticTemplateHost.define(tag);
}

/** Whether metadata proves there is no DOM or reactive work. */
function templateIsNoopHost(meta: TemplateMeta): boolean {
  return meta.h.length === 0
    && meta.tx === undefined
    && meta.a === undefined
    && meta.c === undefined
    && meta.r === undefined
    && meta.eg === undefined
    && meta.b === undefined
    && meta.re === undefined
    && meta.tr === undefined
    && meta.sd === undefined;
}

/** Define a dormant host for one compiler-owned template tag when safe. */
function defineMissingTemplateHost(tag: string, meta: TemplateMeta): void {
  if (
    !templateNeedsStaticHost(meta) ||
    window.__webui?.templateHostExclusions?.has(tag) ||
    customElements.get(tag)
  ) {
    return;
  }
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

  // Claim whatever templates are already registered immediately — streamed
  // boundaries register templates (and may need dormant hosts defined) long
  // before `DOMContentLoaded`. The listener above claims templates that
  // arrive later; this call claims anything already present right now.
  defineTemplateHosts();

  if (document.readyState === 'loading') {
    document.addEventListener(
      'DOMContentLoaded',
      () => defineTemplateHosts(),
      { once: true },
    );
  }
}
