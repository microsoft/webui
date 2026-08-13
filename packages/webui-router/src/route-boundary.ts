// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Select pending/error boundary hints from SSR-emitted route placeholders.
 * The server-provided chain remains authoritative for route content.
 */

import { getRouteMeta, ROUTE_SELECTOR } from './route-element.js';
import type { RouteChainEntry } from './cache.js';

export interface RouteBoundary {
  component: string;
  container: Element | ShadowRoot;
  keepAlive: boolean;
}

export interface BoundaryPathMatch {
  consumed: number;
  specificity: number;
}

interface BoundaryRouteMatch {
  route: HTMLElement;
  container: Element | ShadowRoot;
  consumed: number;
  keepAlive: boolean;
}

export function findPendingComponent(
  activeChain: RouteChainEntry[],
  requestPath: string,
): RouteBoundary | null {
  return findRouteBoundary(activeChain, requestPath, 'pending', 'pendingComponent');
}

export function findErrorComponent(
  activeChain: RouteChainEntry[],
  requestPath: string,
): RouteBoundary | null {
  return findRouteBoundary(activeChain, requestPath, 'error', 'errorComponent');
}

function findRouteBoundary(
  activeChain: RouteChainEntry[],
  requestPath: string,
  attribute: 'pending' | 'error',
  chainProperty: 'pendingComponent' | 'errorComponent',
): RouteBoundary | null {
  const requestSegments = splitBoundaryPath(requestPath);
  const match = { consumed: 0, specificity: 0 };
  let parentIndex = -1;
  let base = 0;
  let lastSelected: BoundaryRouteMatch | null = null;

  for (let level = 0; level <= activeChain.length; level++) {
    const selected = findBestBoundaryRoute(
      activeChain,
      parentIndex,
      requestSegments,
      base,
      match,
    );
    if (!selected) break;
    const activeEntry = activeChain[level];
    if (!activeEntry?.el || selected.route !== activeEntry.el) {
      const component = selected.route.getAttribute(attribute);
      if (component) {
        return {
          component,
          container: selected.container,
          keepAlive: selected.keepAlive,
        };
      }
      return findInheritedBoundary(
        activeChain,
        parentIndex,
        chainProperty,
        selected.container,
        selected.keepAlive,
      );
    }
    lastSelected = selected;
    base = selected.consumed;
    parentIndex = level;
  }

  if (!lastSelected) return null;
  const component = lastSelected.route.getAttribute(attribute);
  if (component) {
    return {
      component,
      container: lastSelected.container,
      keepAlive: lastSelected.keepAlive,
    };
  }
  return findInheritedBoundary(
    activeChain,
    parentIndex,
    chainProperty,
    lastSelected.container,
    lastSelected.keepAlive,
  );
}

function findInheritedBoundary(
  activeChain: RouteChainEntry[],
  startIndex: number,
  chainProperty: 'pendingComponent' | 'errorComponent',
  container: Element | ShadowRoot,
  keepAlive: boolean,
): RouteBoundary | null {
  for (let i = startIndex; i >= 0; i--) {
    const component = activeChain[i][chainProperty];
    if (component) {
      return {
        component,
        container,
        keepAlive,
      };
    }
  }
  return null;
}

function findBestBoundaryRoute(
  activeChain: RouteChainEntry[],
  parentIndex: number,
  requestSegments: string[],
  base: number,
  match: BoundaryPathMatch,
): BoundaryRouteMatch | null {
  const parent = parentIndex >= 0 ? activeChain[parentIndex] : null;
  let root: Element | ShadowRoot = document.body;
  if (parent) {
    if (!parent.el) return null;
    const component = parent.compEl ?? parent.el.querySelector(parent.component);
    if (!component) return null;
    root = (component as HTMLElement).shadowRoot ?? component;
  }

  const routes = root.querySelectorAll<HTMLElement>(ROUTE_SELECTOR);
  let best: BoundaryRouteMatch | null = null;
  let bestSpecificity = -1;
  let ambiguous = false;
  for (let i = 0; i < routes.length; i++) {
    const route = routes[i];
    const ancestorRoute = route.parentElement?.closest(ROUTE_SELECTOR);
    if (ancestorRoute && ancestorRoute !== parent?.el) continue;
    if (!matchBoundaryPath(
      route.getAttribute('path') ?? '',
      route.hasAttribute('exact'),
      requestSegments,
      base,
      match,
    )) {
      continue;
    }
    if (match.specificity > bestSpecificity) {
      best = {
        route,
        container: route.parentElement ?? root,
        consumed: match.consumed,
        keepAlive:
          route.hasAttribute('keep-alive') ||
          getRouteMeta(route)?.keepAlive === true,
      };
      bestSpecificity = match.specificity;
      ambiguous = false;
    } else if (match.specificity === bestSpecificity) {
      ambiguous = true;
    }
  }
  return ambiguous ? null : best;
}

export function splitBoundaryPath(requestPath: string): string[] {
  const queryIndex = requestPath.indexOf('?');
  const path = queryIndex < 0 ? requestPath : requestPath.slice(0, queryIndex);
  const segments: string[] = [];
  for (const segment of path.split('/')) {
    if (segment) segments.push(segment);
  }
  return segments;
}

export function matchBoundaryPath(
  path: string,
  exact: boolean,
  requestSegments: string[],
  base: number,
  result: BoundaryPathMatch,
): boolean {
  const relative = path.length === 0 || !path.startsWith('/');
  const normalized = path.startsWith('./') ? path.slice(2) : path;
  const patterns = normalized.split('/');
  let requestIndex = relative ? base : 0;
  let specificity = relative ? base : 0;

  for (let i = 0; i < patterns.length; i++) {
    const pattern = patterns[i];
    if (!pattern) continue;
    if (pattern[0] === '*') {
      for (; requestIndex < requestSegments.length; requestIndex++) {
        if (!isValidBoundaryParam(requestSegments[requestIndex])) return false;
      }
      continue;
    }
    if (pattern[0] === ':') {
      const optional = pattern.endsWith('?');
      if (requestIndex >= requestSegments.length) {
        if (optional) continue;
        return false;
      }
      if (!isValidBoundaryParam(requestSegments[requestIndex])) return false;
      requestIndex++;
      continue;
    }
    if (requestIndex >= requestSegments.length || requestSegments[requestIndex] !== pattern) {
      return false;
    }
    specificity++;
    requestIndex++;
  }

  if (exact && requestIndex < requestSegments.length) return false;
  result.consumed = requestIndex;
  result.specificity = specificity;
  return true;
}

function isValidBoundaryParam(segment: string): boolean {
  let decoded = segment;
  if (segment.includes('%')) {
    try {
      decoded = decodeURIComponent(segment);
    } catch {
      return false;
    }
  }
  return decoded !== '..' && !decoded.includes('\0');
}
