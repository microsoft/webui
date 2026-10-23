// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { isExact, renderRoot, routeComponent, routePath, ROUTE_SELECTOR } from './route-element.js';
import type { RouteChainEntry } from './cache.js';

/** Opt-in development warnings for route declarations inconsistent with their outlets. */
export function validateRoutes(chain: RouteChainEntry[]): void {
  for (const { el } of chain) {
    if (!el) continue;
    const component = routeComponent(el);
    if (!component) continue;
    const instance = el.querySelector(component);
    if (!instance) continue;
    const root = renderRoot(instance);
    const hasOutlet = root.querySelector('outlet') !== null;
    const hasChildren = root.querySelector(ROUTE_SELECTOR) !== null;
    const path = routePath(el);
    if (!hasChildren && !hasOutlet && !isExact(el) && path !== '/') {
      console.warn(
        `[Router Dev] Route "${path}" (${component}) is a leaf route without "exact". ` +
        'Add "exact" to prevent unintended prefix matching.',
      );
    }
    if (hasOutlet && isExact(el)) {
      console.warn(
        `[Router Dev] Route "${path}" (${component}) has <outlet /> with "exact". ` +
        'Remove "exact" - child routes will never match.',
      );
    }
  }
}
