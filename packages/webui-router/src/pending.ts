// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Pending & error boundary UI — manages pending/loading and error
 * components shown during navigation.
 */

import { isStateful } from './types.js';
import { ROUTE_SELECTOR } from './route-element.js';
import type { RouteChainEntry } from './cache.js';

/**
 * Find the pending component for a target route.
 * Walks active chain looking for `pendingComponent`, preferring the deepest match.
 * Falls back to scanning SSR'd stubs scoped to the deepest active leaf's children.
 */
export function findPendingComponent(
  activeChain: RouteChainEntry[],
  _requestPath: string,
): string | null {
  // Check active chain (parent routes that already have metadata from partial)
  for (let i = activeChain.length - 1; i >= 0; i--) {
    if (activeChain[i].pendingComponent) {
      return activeChain[i].pendingComponent!;
    }
  }
  // Walk SSR'd route stubs scoped to the deepest active leaf's children
  const leaf = activeChain[activeChain.length - 1];
  if (leaf?.el) {
    const compEl = leaf.compEl ?? leaf.el.querySelector(leaf.component);
    if (compEl) {
      const root = (compEl as HTMLElement).shadowRoot ?? compEl;
      for (const el of root.querySelectorAll(ROUTE_SELECTOR)) {
        const pending = el.getAttribute('pending');
        if (pending) return pending;
      }
    }
  }
  return null;
}

/**
 * Find the error component for a target route.
 * Same scoping strategy as findPendingComponent.
 */
export function findErrorComponent(
  activeChain: RouteChainEntry[],
  _requestPath: string,
): string | null {
  for (let i = activeChain.length - 1; i >= 0; i--) {
    if (activeChain[i].errorComponent) {
      return activeChain[i].errorComponent!;
    }
  }
  const leaf = activeChain[activeChain.length - 1];
  if (leaf?.el) {
    const compEl = leaf.compEl ?? leaf.el.querySelector(leaf.component);
    if (compEl) {
      const root = (compEl as HTMLElement).shadowRoot ?? compEl;
      for (const el of root.querySelectorAll(ROUTE_SELECTOR)) {
        const error = el.getAttribute('error');
        if (error) return error;
      }
    }
  }
  return null;
}

/** State holder for pending/error elements — tracks mounted elements for O(1) cleanup. */
export class PendingState {
  pendingElement: HTMLElement | null = null;
  errorElement: HTMLElement | null = null;
  pendingTimer: ReturnType<typeof setTimeout> | null = null;

  /** Clear the pending UI timer. */
  clearTimer(): void {
    if (this.pendingTimer) {
      clearTimeout(this.pendingTimer);
      this.pendingTimer = null;
    }
  }

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
    this.clearTimer();
    this.pendingElement = null;
    this.errorElement = null;
  }
}


