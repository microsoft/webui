// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Route-element helpers — constants, DOM accessors, and the
 * `<webui-route>` custom element extracted from the monolithic router.
 */

import { isStateful } from './types.js';
import type { StatefulElement } from './types.js';

// ── Constants ────────────────────────────────────────────────────

export const ROUTE_SELECTOR = 'webui-route';

// ── Module-scope state ───────────────────────────────────────────

/** Type-safe route param storage — avoids expando properties on DOM elements. */
const routeParamsMap = new WeakMap<Element, Record<string, string>>();

/** Metadata associated with a route element from chain data. */
export interface RouteMeta {
  allowedQuery: string | undefined;
  keepAlive: boolean;
}

/** Route metadata populated from the SSR chain or server partial responses. */
const routeMetaMap = new WeakMap<Element, RouteMeta>();

/** Store route metadata on an element (clears any cached query allowlist). */
export function setRouteMeta(el: Element, meta: RouteMeta): void {
  routeMetaMap.set(el, meta);
  allowedQueryCache.delete(el);
}

/** Retrieve route metadata previously stored via {@link setRouteMeta}. */
export function getRouteMeta(el: Element): RouteMeta | undefined {
  return routeMetaMap.get(el);
}

/**
 * Tracks which query-param attribute names (kebab-case) were last applied to
 * each component element. Used to remove stale attributes when query params
 * change (e.g. navigating from `?subject=foo` to no `subject`).
 */
const queryAttrsMap = new WeakMap<Element, Set<string>>();

/** Cached parsed query allowlist per route element — avoids re-splitting on every navigation. */
const allowedQueryCache = new WeakMap<Element, Set<string> | null>();

// ── Free functions ───────────────────────────────────────────────

/**
 * Check if a state value is meaningful (non-null, non-empty).
 * Returns false for null, undefined, or `{}`.
 */
export function hasState(state?: Record<string, unknown> | null): state is Record<string, unknown> {
  if (state == null) return false;
  const keys = Object.keys(state);
  return keys.length > 0;
}

/**
 * Get the render root of a component element.
 * Returns shadowRoot if present, otherwise the element itself.
 * This allows the router to work in both shadow and light DOM modes.
 */
export function renderRoot(el: Element): Element | ShadowRoot {
  return (el as HTMLElement).shadowRoot ?? el;
}

/** Create a hidden `<webui-route>` stub element. */
export function createRouteStub(entry: { path?: string; component?: string; exact?: boolean }): HTMLElement {
  const el = document.createElement(ROUTE_SELECTOR);
  if (entry.path) el.setAttribute('path', entry.path);
  if (entry.component) el.setAttribute('component', entry.component);
  if (entry.exact) el.setAttribute('exact', '');
  el.style.display = 'none';
  return el;
}

// ── Route element helpers ────────────────────────────────────────

export function routePath(el: Element): string {
  return el.getAttribute('path') ?? '';
}

export function isExact(el: Element): boolean {
  return el.hasAttribute('exact');
}

export function routeComponent(el: Element): string {
  return el.getAttribute('component') ?? '';
}

export function getRouteParams(el: Element): Record<string, string> {
  return routeParamsMap.get(el) ?? {};
}

/** Convert a camelCase key to a kebab-case attribute name. */
export const toKebab = (k: string): string => k.replace(/[A-Z]/g, m => `-${m.toLowerCase()}`);

/** Parse query-string parameters from a request path (e.g. `/compose?action=reply&to=x`). */
export function parseQuery(requestPath: string): Record<string, string> {
  const qIdx = requestPath.indexOf('?');
  if (qIdx < 0) return {};
  const query: Record<string, string> = {};
  const params = new URLSearchParams(requestPath.slice(qIdx));
  for (const [k, v] of params) {
    query[k] = v;
  }
  return query;
}

/**
 * Read the comma-separated `query` allowlist for a route element.
 * Checks the {@link RouteMeta} WeakMap first (populated from chain data),
 * then falls back to the DOM `query` attribute (SSR stubs not yet in chain).
 * Returns null if no allowlist is configured (deny-by-default).
 */
export function routeAllowedQuery(el: Element): Set<string> | null {
  const cached = allowedQueryCache.get(el);
  if (cached !== undefined) return cached;

  // Check WeakMap first (populated from chain data)
  const meta = getRouteMeta(el);
  const raw = meta?.allowedQuery ?? el.getAttribute('query');

  if (raw == null) {
    allowedQueryCache.set(el, null);
    return null;
  }
  const set = new Set<string>();
  for (const part of raw.split(',')) {
    const trimmed = part.trim();
    if (trimmed) set.add(trimmed);
  }
  allowedQueryCache.set(el, set);
  return set;
}

/**
 * Filter query params through an allowlist. Returns only key-value pairs
 * whose keys appear in `allowed`. If `allowed` is null (no `query` attr
 * on the route), returns an empty object (deny-by-default).
 *
 * Keys whose kebab-case form collides with a route param's kebab-case
 * form are always excluded so that path parameters cannot be overridden
 * via query string.
 */
export function filterQuery(
  query: Record<string, string>,
  allowed: Set<string> | null,
  routeParams?: Record<string, string>,
): Record<string, string> {
  if (!allowed) return {};
  // Build a set of kebab-cased route param attribute names for collision check
  let paramAttrNames: Set<string> | undefined;
  if (routeParams) {
    paramAttrNames = new Set<string>();
    const rpKeys = Object.keys(routeParams);
    for (let i = 0; i < rpKeys.length; i++) paramAttrNames.add(toKebab(rpKeys[i]));
  }
  const result: Record<string, string> = {};
  const qKeys = Object.keys(query);
  for (let i = 0; i < qKeys.length; i++) {
    const k = qKeys[i];
    if (allowed.has(k) && !(paramAttrNames && paramAttrNames.has(toKebab(k)))) {
      result[k] = query[k];
    }
  }
  return result;
}

export function activateRoute(el: HTMLElement, params: Record<string, string>): void {
  routeParamsMap.set(el, params);
  el.setAttribute('active', '');
  el.style.display = '';
}

export function deactivateRoute(el: HTMLElement): void {
  routeParamsMap.set(el, {});
  el.removeAttribute('active');
  el.style.display = 'none';
}

// ── Compound helpers ─────────────────────────────────────────────

/**
 * Apply route params and query params as HTML attributes on a component.
 * Does NOT call setState — used for keep-alive reactivation where local
 * state should be preserved. Stale query-param attributes from a previous
 * navigation are automatically removed.
 */
export function applyParamsAndQuery(
  component: Element,
  routeEl: HTMLElement,
  params: Record<string, string>,
  query?: Record<string, string>,
): void {
  const paramKeys = Object.keys(params);
  for (let i = 0; i < paramKeys.length; i++) {
    component.setAttribute(toKebab(paramKeys[i]), params[paramKeys[i]]);
  }

  const allowed = routeAllowedQuery(routeEl);
  if (!allowed || !query) {
    // Fast path: no query params to process — just clean up stale attrs
    const prevAttrs = queryAttrsMap.get(component);
    if (prevAttrs) {
      for (const attr of prevAttrs) component.removeAttribute(attr);
      queryAttrsMap.delete(component);
    }
    return;
  }

  const filtered = filterQuery(query, allowed, params);
  const newAttrs = new Set<string>();
  const filteredKeys = Object.keys(filtered);
  for (let i = 0; i < filteredKeys.length; i++) {
    const key = filteredKeys[i];
    const attr = toKebab(key);
    component.setAttribute(attr, filtered[key]);
    newAttrs.add(attr);
  }

  const prevAttrs = queryAttrsMap.get(component);
  if (prevAttrs) {
    for (const attr of prevAttrs) {
      if (!newAttrs.has(attr)) {
        component.removeAttribute(attr);
      }
    }
  }
  if (newAttrs.size > 0) {
    queryAttrsMap.set(component, newAttrs);
  } else {
    queryAttrsMap.delete(component);
  }
}

/**
 * Apply route params, allowed query params, and state to a component.
 * Shared by both initial mount and subsequent state updates. Stale query-param
 * attributes from a previous navigation are automatically removed.
 *
 * For keep-alive reactivation without a loader, use {@link applyParamsAndQuery}
 * instead — it updates attributes without overwriting component state.
 */
export function applyParamsQueryState(
  component: Element,
  routeEl: HTMLElement,
  params: Record<string, string>,
  state?: Record<string, unknown> | null,
  query?: Record<string, string>,
): void {
  applyParamsAndQuery(component, routeEl, params, query);

  if (hasState(state) && isStateful(component)) {
    component.setState(state);
  }
}

// ── WebUIRouteElement custom element ─────────────────────────────

/** Custom element backing `<webui-route>`. */
export class WebUIRouteElement extends HTMLElement {
  get path(): string { return this.getAttribute('path') ?? ''; }
  get exact(): boolean { return this.hasAttribute('exact'); }
  get component(): string { return this.getAttribute('component') ?? ''; }
  get isActive(): boolean { return this.hasAttribute('active'); }
  get keepAlive(): boolean { return this.hasAttribute('keep-alive'); }
  get params(): Record<string, string> { return getRouteParams(this); }
  /** Comma-separated allowlist of query params forwarded as attributes. */
  get query(): string { return this.getAttribute('query') ?? ''; }
}
