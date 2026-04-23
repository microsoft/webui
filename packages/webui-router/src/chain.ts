// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Route chain building & reconciliation — builds the active route chain
 * from SSR'd DOM elements and diffs old/new chains to find the first
 * changed level for efficient partial re-rendering.
 */

import {
  ROUTE_SELECTOR,
  routePath,
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
 * Primary path: reads the `window.__webui.chain` array emitted
 * by the Rust handler during SSR.  Each entry already contains the
 * resolved `component`, `path`, `params`, etc.
 *
 * Fallback: if `window.__webui` is not available, tries the legacy
 * `<script id="webui-chain">` JSON blob, then walks the DOM tree of
 * active `<webui-route>` elements.
 */
export function buildChainFromSSR(): RouteChainEntry[] {
  // Primary: read from bundled window.__webui object
  const meta = window.__webui;
  if (meta?.chain && Array.isArray(meta.chain)) {
    return hydrateChainFromJSON(meta.chain as RouteChainEntry[]);
  }

  // Fallback: try legacy script tag approach
  const scriptEl = document.querySelector<HTMLElement>('#webui-chain');
  const raw = scriptEl?.textContent;
  if (raw) {
    try {
      const entries = JSON.parse(raw) as RouteChainEntry[];
      return hydrateChainFromJSON(entries);
    } catch {
      // Malformed JSON — fall through to DOM-walk fallback
    }
  }

  // Final fallback: walk SSR'd active routes in the DOM (pre-chain servers)
  return buildChainFromDOM();
}

/**
 * Hydrate a chain parsed from the server-emitted JSON.
 * Uses `data-ri` attributes for O(1) element binding when available,
 * falling back to component-name matching for legacy SSR output.
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

  const hasRiAttrs = riMap.size > 0;
  const chain: RouteChainEntry[] = [];

  if (hasRiAttrs) {
    // Fast path: O(1) lookup by chain index
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
  } else {
    // Legacy fallback: walk DOM by component name
    let currentRoutes = discoverTopRoutes();

    for (const entry of entries) {
      entry.params = entry.params ?? {};
      entry.component = entry.component ?? '';
      entry.path = entry.path ?? '';

      const el = findRouteByComponent(currentRoutes, entry.component);
      if (el) {
        entry.el = el;
        activateRoute(el, entry.params);
        setRouteMeta(el, {
          allowedQuery: entry.allowedQuery,
          keepAlive: entry.keepAlive ?? false,
        });
        chain.push(entry);
        currentRoutes = discoverChildRoutes(el);
      } else {
        chain.push(entry);
      }
    }
  }

  return chain;
}

/** Find a route element by its `component` attribute within a set of candidates. */
function findRouteByComponent(routes: HTMLElement[], component: string): HTMLElement | undefined {
  for (const el of routes) {
    if (routeComponent(el) === component) return el;
  }
  return undefined;
}

/**
 * DOM-walk fallback for servers that don't emit `#webui-chain`.
 * Walks active `<webui-route>` elements without param extraction
 * (params are empty — acceptable because the SSR content is already rendered).
 */
function buildChainFromDOM(): RouteChainEntry[] {
  const chain: RouteChainEntry[] = [];
  let currentRoutes = discoverTopRoutes();

  while (currentRoutes.length > 0) {
    const activeEl = currentRoutes.find(el => el.hasAttribute('active'));
    if (!activeEl) break;

    const comp = routeComponent(activeEl);
    const rawPath = routePath(activeEl);
    const params: Record<string, string> = {};

    const entry: RouteChainEntry = {
      component: comp,
      path: rawPath,
      params,
      exact: isExact(activeEl),
      el: activeEl,
      allowedQuery: activeEl.getAttribute('query') ?? undefined,
      keepAlive: activeEl.hasAttribute('keep-alive'),
      pendingComponent: activeEl.getAttribute('pending') ?? undefined,
      errorComponent: activeEl.getAttribute('error') ?? undefined,
      invalidates: activeEl.getAttribute('invalidates')
        ?.split(',').map(s => s.trim()).filter(Boolean) ?? undefined,
    };

    chain.push(entry);
    setRouteMeta(activeEl, {
      allowedQuery: entry.allowedQuery,
      keepAlive: entry.keepAlive ?? false,
    });
    activateRoute(activeEl, params);

    currentRoutes = discoverChildRoutes(activeEl);
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
