// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Pending & error boundary UI — manages pending/loading and error
 * components shown during navigation.
 */

import { isStateful } from './types.js';
import { ROUTE_SELECTOR } from './route-element.js';
import type { RouteChainEntry } from './cache.js';

/** State holder for pending/error elements — tracks mounted elements for O(1) cleanup. */
export class PendingState {
  pendingElement: HTMLElement | null = null;
  errorElement: HTMLElement | null = null;

  /** Remove any pending/error elements left over from a previous navigation. */
  clearElements(): void {
    if (this.pendingElement) {
      this.pendingElement.remove();
      this.pendingElement = null;
    }
    if (this.errorElement) {
      this.errorElement.remove();
      this.errorElement = null;
    }
  }

  /**
   * Mount a pending/loading component in the outlet area.
   * Finds the target route's parent (deepest active leaf) and appends
   * the pending component in its outlet container.
   */
  mountPending(componentTag: string, activeChain: RouteChainEntry[]): void {
    const leaf = activeChain[activeChain.length - 1];
    if (!leaf?.el) return;

    // Don't show pending for keep-alive routes (they activate instantly)
    if (leaf.keepAlive) return;

    const existing = leaf.el.querySelector(componentTag);
    if (existing) return; // Already showing

    // Mount inside the leaf's component's outlet area (where child routes go)
    const compEl = leaf.compEl ?? leaf.el.querySelector(leaf.component);
    if (!compEl) return;

    const root = (compEl as HTMLElement).shadowRoot ?? compEl;

    // Find existing sibling route elements or an outlet marker
    const siblingRoutes = root.querySelectorAll(ROUTE_SELECTOR);
    const container = siblingRoutes.length > 0
      ? siblingRoutes[siblingRoutes.length - 1].parentElement
      : (root.querySelector('outlet')?.parentElement ?? root);
    if (!container) return;

    const pending = document.createElement(componentTag);
    pending.setAttribute('data-webui-pending', '');
    container.appendChild(pending);
    this.pendingElement = pending;
  }

  /**
   * Mount an error boundary component in the outlet area.
   * Passes error details as state.
   */
  mountError(
    componentTag: string,
    errorState: { error: string; status: number; path: string },
    activeChain: RouteChainEntry[],
  ): void {
    const leaf = activeChain[activeChain.length - 1];
    if (!leaf?.el) return;

    const compEl = leaf.compEl ?? leaf.el.querySelector(leaf.component);
    if (!compEl) return;

    const root = (compEl as HTMLElement).shadowRoot ?? compEl;

    // Find existing sibling route elements or an outlet marker
    const siblingRoutes = root.querySelectorAll(ROUTE_SELECTOR);
    const container = siblingRoutes.length > 0
      ? siblingRoutes[siblingRoutes.length - 1].parentElement
      : (root.querySelector('outlet')?.parentElement ?? root);
    if (!container) return;

    // Hide all existing route children
    for (const child of container.querySelectorAll(ROUTE_SELECTOR)) {
      (child as HTMLElement).style.display = 'none';
    }

    const errorEl = document.createElement(componentTag);
    errorEl.setAttribute('data-webui-error', '');
    container.appendChild(errorEl);
    this.errorElement = errorEl;
    if (isStateful(errorEl)) {
      errorEl.setState(errorState);
    }
  }

  /** Clean up all pending state. */
  destroy(): void {
    this.pendingElement = null;
    this.errorElement = null;
  }
}

