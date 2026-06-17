// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Route chain building & reconciliation — builds the active route chain
 * from SSR'd DOM elements and diffs old/new chains to find the first
 * changed level for efficient partial re-rendering.
 */

import {
  ROUTE_SELECTOR,
  isExact,
  routeComponent,
  activateRoute,
  renderRoot,
  createRouteStub,
  setRouteMeta,
} from './route-element.js';
import type { RouteChainEntry } from './cache.js';

/**
 * Build the initial route chain from SSR data.
 *
 * Reads the `window.__webui.chain` array loaded from `#webui-data`.
 * Each entry already contains the resolved `component`, `path`, `params`, etc.
 */
export function buildChainFromSSR(): RouteChainEntry[] {
  const meta = window.__webui;
  if (meta?.chain && Array.isArray(meta.chain)) {
    return hydrateChainFromJSON(meta.chain as RouteChainEntry[]);
  }
  return [];
}

/**
 * Hydrate a chain parsed from the server-emitted JSON.
 * Uses `data-ri` attributes for O(1) element binding.
 */
function hydrateChainFromJSON(entries: RouteChainEntry[]): RouteChainEntry[] {
  // Collect all indexed route elements, piercing shadow roots.
  // With Declarative Shadow DOM, route elements are nested inside shadow
  // boundaries so document.querySelectorAll alone can't find them.
  const riMap = new Map<number, HTMLElement>();
  function collectRi(root: Document | ShadowRoot): void {
    for (const el of root.querySelectorAll<HTMLElement>('[data-ri]')) {
      const ri = parseInt(el.getAttribute('data-ri')!, 10);
      if (!isNaN(ri)) riMap.set(ri, el);
    }
    for (const el of root.querySelectorAll<HTMLElement>('*')) {
      if (el.shadowRoot) collectRi(el.shadowRoot);
    }
  }
  collectRi(document);

  const chain: RouteChainEntry[] = [];

  for (let i = 0; i < entries.length; i++) {
    const entry = entries[i];
    entry.params = entry.params ?? {};
    entry.component = entry.component ?? '';
    entry.path = entry.path ?? '';

    const el = riMap.get(i);
    if (el) {
      entry.el = el;
      activateRoute(el, entry.params);
      setRouteMeta(el, {
        allowedQuery: entry.allowedQuery,
        keepAlive: entry.keepAlive ?? false,
      });
    }
    chain.push(entry);
  }
  return chain;
}

/**
 * Compare old and new chains to find the first level that differs.
 * Returns the index of the first changed level.
 */
export function findChangeLevel(
  oldChain: RouteChainEntry[],
  newChain: RouteChainEntry[],
): number {
  const len = Math.min(oldChain.length, newChain.length);
  for (let i = 0; i < len; i++) {
    if (
      oldChain[i].component !== newChain[i].component ||
      !paramsEqual(oldChain[i].params, newChain[i].params)
    ) {
      return i;
    }
  }
  // If chains differ in length, change starts at the shorter length
  return len;
}

/**
 * Find or create a `<webui-route>` DOM element for a chain entry.
 * For top-level routes, searches direct children of `<body>`.
 * For nested routes, searches the parent component's render root
 * (shadow root or light DOM).
 */
export function findOrCreateRouteElement(
  parent: RouteChainEntry | null,
  entry: RouteChainEntry,
): HTMLElement {
  // For top-level routes, search direct children of body
  if (!parent) {
    for (const child of document.body.children) {
      if (child.tagName === 'WEBUI-ROUTE' &&
          child.getAttribute('component') === entry.component) {
        return child as HTMLElement;
      }
    }
    const el = createRouteStub(entry);
    document.body.appendChild(el);
    return el;
  }

  // For nested routes, search in parent component's render root
  if (parent.el) {
    const compEl = parent.compEl ?? parent.el.querySelector(parent.component);
    if (compEl) {
      const root = renderRoot(compEl);
      const allRoutes = root.querySelectorAll(ROUTE_SELECTOR);

      // Match by component + path for stronger identity
      for (const child of allRoutes) {
        if (child.getAttribute('component') === entry.component &&
            child.getAttribute('path') === (entry.path || null)) {
          return child as HTMLElement;
        }
      }
      // Fallback: match by component only (backwards compat)
      for (const child of allRoutes) {
        if (child.getAttribute('component') === entry.component) {
          return child as HTMLElement;
        }
      }

      // Not found — create stub and place in the correct container
      const stub = createRouteStub(entry);

      // Strategy 1: use the parent container of existing sibling routes.
      if (allRoutes.length > 0) {
        const container = allRoutes[allRoutes.length - 1].parentElement;
        if (container) {
          container.appendChild(stub);
          return stub;
        }
      }

      // Strategy 2: insert after the <outlet> marker (f-template components)
      const outletMarker = root.querySelector('outlet');
      if (outletMarker?.parentElement) {
        outletMarker.parentElement.insertBefore(stub, outletMarker.nextSibling);
        return stub;
      }

      // Strategy 3: fallback — append to render root
      root.appendChild(stub);
      return stub;
    }
  }

  // Fallback: create and append to body
  const el = createRouteStub(entry);
  document.body.appendChild(el);
  return el;
}

/** Compare two param objects for equality. */
export function paramsEqual(
  a: Record<string, string>,
  b: Record<string, string>,
): boolean {
  const aKeys = Object.keys(a);
  if (aKeys.length !== Object.keys(b).length) return false;
  for (let i = 0; i < aKeys.length; i++) {
    if (a[aKeys[i]] !== b[aKeys[i]]) return false;
  }
  return true;
}

/**
 * Find top-level route elements — direct children of `<body>`.
 */
export function discoverTopRoutes(): HTMLElement[] {
  const results: HTMLElement[] = [];
  for (const child of document.body.children) {
    if (child.tagName === 'WEBUI-ROUTE') {
      results.push(child as HTMLElement);
    }
  }
  return results;
}

/**
 * Find child route elements inside a parent route's component.
 * Traverses: parent route → component → component's render root → `<webui-route>` elements.
 */
export function discoverChildRoutes(parentRoute: HTMLElement): HTMLElement[] {
  const results: HTMLElement[] = [];
  const comp = routeComponent(parentRoute);
  if (!comp) return results;

  const compEl = parentRoute.querySelector(comp);
  if (!compEl) return results;

  const root = renderRoot(compEl);
  for (const child of root.querySelectorAll(ROUTE_SELECTOR)) {
    results.push(child as HTMLElement);
  }

  return results;
}
