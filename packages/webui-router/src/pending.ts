// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Pending & error boundary UI — manages pending/loading and error
 * components shown during navigation.
 */

import { isStateful } from './types.js';

/** State holder for pending/error elements — tracks mounted elements for O(1) cleanup. */
export class PendingState {
  pendingElement: HTMLElement | null = null;
  errorElement: HTMLElement | null = null;
  private hiddenRouteDisplays: Map<HTMLElement, string> | null = null;

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
    if (this.hiddenRouteDisplays) {
      for (const [route, display] of this.hiddenRouteDisplays) {
        route.style.display = display;
      }
      this.hiddenRouteDisplays = null;
    }
  }

  /**
   * Mount a pending/loading component in the outlet area.
   * Finds the target route's parent (deepest active leaf) and appends
   * the pending component in its outlet container.
   */
  mountPending(
    componentTag: string,
    container: Element | ShadowRoot,
  ): void {
    if (this.pendingElement?.isConnected) return;

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
    container: Element | ShadowRoot,
  ): void {
    // Hide all existing route children
    const hiddenRouteDisplays = new Map<HTMLElement, string>();
    const children = container.children;
    for (let i = 0; i < children.length; i++) {
      const child = children[i];
      if (child.tagName !== 'WEBUI-ROUTE') continue;
      const route = child as HTMLElement;
      hiddenRouteDisplays.set(route, route.style.display);
      route.style.display = 'none';
    }
    this.hiddenRouteDisplays = hiddenRouteDisplays;

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
    this.clearElements();
  }
}
