// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Form interception — intercepts `<form method="post">` submissions
 * and delegates to the nearest route component's `static action()`.
 */

import { isStateful } from './types.js';
import type { RouteActionContext, RouteActionResult, ActionCompleteEvent } from './types.js';
import { getRouteParams } from './route-element.js';
import type { RouteChainEntry } from './cache.js';

/** Context needed by form interception to interact with router state. */
export interface ActionContext {
  readonly activeChain: RouteChainEntry[];
  readonly currentRequestPath: string;
  setActionController(controller: AbortController | null): void;
  invalidateTags(tags: string[]): void;
}

/**
 * Set up delegated form submission interception.
 * Returns a cleanup function to remove the listener.
 */
export function setupFormInterception(ctx: ActionContext): () => void {
  const onSubmit = (e: SubmitEvent): void => {
    // Walk composedPath to find the form — works across shadow boundaries
    const path = e.composedPath();
    let form: HTMLFormElement | undefined;
    for (let i = 0; i < path.length; i++) {
      const el = path[i] as Element;
      if (el?.tagName === 'FORM' && (el as HTMLFormElement).method?.toLowerCase() === 'post') {
        form = el as HTMLFormElement;
        break;
      }
    }
    if (!form) return;

    // Only intercept forms without an explicit action or targeting same-origin.
    // Forms with external action URLs (payment, auth, etc.) must not be hijacked.
    const formAction = form.action; // resolved absolute URL
    if (formAction) {
      try {
        const actionUrl = new URL(formAction);
        if (actionUrl.origin !== location.origin) return;
      } catch {
        return; // malformed action — don't intercept
      }
    }
    // Forms with a target attribute submit to a different browsing context
    if (form.target && form.target !== '_self') return;

    // Find the nearest ancestor <webui-route> with a component
    let routeEl: HTMLElement | null = null;
    for (let i = 0; i < path.length; i++) {
      const el = path[i] as Element;
      if (el?.tagName === 'WEBUI-ROUTE' && el.getAttribute('component')) {
        routeEl = el as HTMLElement;
        break;
      }
    }
    if (!routeEl) return;

    const componentTag = routeEl.getAttribute('component');
    if (!componentTag) return;

    // Check if the component has a static action() method
    const ctor = customElements.get(componentTag) as (
      (new () => HTMLElement) & { action?: (ctx: RouteActionContext) => Promise<RouteActionResult | void> }
    ) | undefined;
    if (!ctor || typeof ctor.action !== 'function') return;

    // Prevent default form submission
    e.preventDefault();

    const formData = new FormData(form);
    const params = getRouteParams(routeEl);
    const controller = new AbortController();
    ctx.setActionController(controller);

    // Get resolved invalidation tags from the active chain entry
    // (not from DOM attr which has unresolved {param} templates)
    const chainEntry = ctx.activeChain.find(e => e.component === componentTag);
    const routeInvalidates = chainEntry?.invalidates ?? [];

    ctor.action({ formData, params, signal: controller.signal })
      .then((result: RouteActionResult | void) => {
        if (controller.signal.aborted) return;

        // Apply optimistic state if provided
        if (result?.state) {
          const compEl = chainEntry?.compEl ?? routeEl!.querySelector(componentTag!);
          if (compEl && isStateful(compEl)) {
            compEl.setState(result.state);
          }
        }

        // Merge action-returned tags with route's build-time invalidates
        const allTags = new Set<string>();
        for (const tag of routeInvalidates) allTags.add(tag);
        if (result?.invalidateTags) {
          for (const tag of result.invalidateTags) allTags.add(tag);
        }
        const mergedTags = [...allTags];

        // Invalidate cache
        if (mergedTags.length > 0) {
          ctx.invalidateTags(mergedTags);
        }

        // Dispatch completion event
        const detail: ActionCompleteEvent = {
          component: componentTag!,
          invalidatedTags: mergedTags,
          path: ctx.currentRequestPath,
        };
        window.dispatchEvent(new CustomEvent('webui:route:action-complete', { detail }));
      })
      .catch((err: unknown) => {
        if (err instanceof DOMException && err.name === 'AbortError') return;
        console.error(`[Router] Action failed for <${componentTag}>:`, err);
      });
  };

  document.addEventListener('submit', onSubmit);
  return () => document.removeEventListener('submit', onSubmit);
}
