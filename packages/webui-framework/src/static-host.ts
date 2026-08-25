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
import {
  claimSsrComponentStyles,
  installComponentStyles,
} from './element/styles.js';
import { consumePendingParentState } from './pending-parent-state.js';
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

/**
 * Restore values a parent wrote before this compiler-owned tag was defined.
 *
 * The default entry installs static hosts in a later task, so an authored
 * parent can deliver a `:` property first. The shared WeakMap keeps the common
 * no-write host field-free; only values that were actually queued become own
 * properties here.
 */
function applyPendingNoopHostState(host: HTMLElement): void {
  const pending = consumePendingParentState(host);
  if (!pending) return;
  const target = host as unknown as Record<string, unknown>;
  const keys = Object.keys(pending.values);
  for (let i = 0; i < keys.length; i++) {
    const key = keys[i];
    target[key] = pending.values[key];
  }
}

/**
 * Preserve component CSS without allocating the full template runtime.
 *
 * Style closures are registered separately from template metadata, so `h: ''`
 * does not imply that the host has no CSS to claim or install.
 */
function installNoopHostStyles(host: HTMLElement, tag: string): void {
  const containingRoot = host.getRootNode();
  const target = containingRoot.nodeType === 11 && 'host' in containingRoot
    ? containingRoot as ShadowRoot
    : host.ownerDocument;
  claimSsrComponentStyles(host, target);
  const installation = installComponentStyles(tag, target, host);
  if (installation) {
    void installation.catch((error) => {
      console.error(error);
    });
  }
}

/** Define the smallest client-rendering element for a compiler-owned template. */
function defineTemplateHost(tag: string, meta: TemplateMeta): void {
  const w = window as Window;
  if (!w.__webui) w.__webui = {};
  if (!w.__webui.templates) w.__webui.templates = {};
  if (!w.__webui.templates[tag]) w.__webui.templates[tag] = meta;

  if (templateIsNoopHost(meta)) {
    // The tag still needs custom-element identity, queued parent properties,
    // component CSS, and the streaming hook. Keeping those behaviors on the
    // prototype avoids TemplateElement's per-instance state for empty hosts.
    customElements.define(tag, class extends HTMLElement {
      connectedCallback(): void {
        // Match TemplateElement timing: an uncommitted streamed root installs
        // its styles when the boundary activates, not while its DOM is partial.
        // Inspect the marker before queued property names can shadow DOM methods.
        if (
          isStreamingHydrationMode() &&
          this.hasAttribute(STREAMED_HOST_ATTR)
        ) {
          return;
        }
        installNoopHostStyles(this, tag);
        applyPendingNoopHostState(this);
      }

      [STREAMING_BOUNDARY_ACTIVATE](): typeof ACTIVATION_STATIC_HOST_OPT_OUT {
        // Style resolution calls DOM methods such as getRootNode(), so complete
        // it before exposing arbitrary queued property names on the instance.
        installNoopHostStyles(this, tag);
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

/**
 * Return true when a compiler-owned host has no DOM or reactive work.
 *
 * The compiler omits empty optional metadata. Any present behavior-bearing
 * field keeps the full TemplateElement path, even when the static HTML is empty.
 */
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
