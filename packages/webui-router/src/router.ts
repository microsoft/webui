// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Core router — uses the Navigation API to intercept navigations and
 * activates/deactivates `<webui-route>` elements in the DOM tree.
 *
 * Supports **nested routes**: routes inside matched route components are
 * resolved relative to their parent's consumed path. Parent content persists
 * when only child routes change.
 *
 * For routes with a `component` attribute, the router fetches a JSON
 * partial from the server (state + f-templates), registers any new
 * templates, instantiates the component, and mounts it into the route.
 */

import { clearInventoryBit, encodeInventoryHex, parseInventoryHex } from './inventory.js';
import { buildNavigationTarget, prependBasePath } from './navigation-path.js';
import type { RouterConfig, NavigationEvent } from './types.js';
import type { NavigationTarget } from './navigation-path.js';

const ROUTE_SELECTOR = 'webui-route';

/** Create a hidden `<webui-route>` stub element. */
function createRouteStub(entry: { path?: string; component?: string; exact?: boolean }): HTMLElement {
  const el = document.createElement(ROUTE_SELECTOR);
  if (entry.path) el.setAttribute('path', entry.path);
  if (entry.component) el.setAttribute('component', entry.component);
  if (entry.exact) el.setAttribute('exact', '');
  el.style.display = 'none';
  return el;
}

// ── Route element helpers ────────────────────────────────────────

function routePath(el: Element): string {
  return el.getAttribute('path') ?? '';
}

function isExact(el: Element): boolean {
  return el.hasAttribute('exact');
}

function routeComponent(el: Element): string {
  return el.getAttribute('component') ?? '';
}

/** Type-safe route param storage — avoids expando properties on DOM elements. */
const routeParamsMap = new WeakMap<Element, Record<string, string>>();

function activateRoute(el: HTMLElement, params: Record<string, string>): void {
  routeParamsMap.set(el, params);
  el.setAttribute('active', '');
  el.style.display = '';
}

function deactivateRoute(el: HTMLElement): void {
  routeParamsMap.set(el, {});
  el.removeAttribute('active');
  el.style.display = 'none';
}

function getRouteParams(el: Element): Record<string, string> {
  return routeParamsMap.get(el) ?? {};
}

// ── WebUIRouteElement custom element ─────────────────────────────

/** Custom element backing `<webui-route>`. */
export class WebUIRouteElement extends HTMLElement {
  get path(): string { return this.getAttribute('path') ?? ''; }
  get exact(): boolean { return this.hasAttribute('exact'); }
  get component(): string { return this.getAttribute('component') ?? ''; }
  get isActive(): boolean { return this.hasAttribute('active'); }
  get params(): Record<string, string> { return getRouteParams(this); }
}

// ── Route Chain Types ────────────────────────────────────────────

/** An entry in the matched route chain, one per nesting level. */
interface RouteChainEntry {
  /** Component tag name for this route level. */
  component: string;
  /** Route path pattern as declared in the template. */
  path: string;
  /** Bound route parameters at this level. */
  params: Record<string, string>;
  /** Whether this route requires an exact match. */
  exact?: boolean;
  /** DOM element, populated during mount or SSR chain build. */
  el?: HTMLElement;
}

// ── Router ───────────────────────────────────────────────────────

/** JSON partial response from the server. */
interface PartialResponse {
  state: Record<string, unknown>;
  templates: string[];
  path: string;
  chain?: RouteChainEntry[];
}

export class WebUIRouter {
  private config: RouterConfig = {};
  private started = false;
  private cleanupFns: Array<() => void> = [];
  private isInitialNavigation = true;
  /** Hex string tracking which component templates are loaded. */
  private inventory = '';
  /** Opt-in lazy loaders: component tag → async import function. */
  private loaders: Record<string, () => Promise<unknown>> = {};
  /** Deduplication cache for in-flight / completed loader promises. */
  private loaderPromises = new Map<string, Promise<void>>();
  /** Current active route chain for reconciliation on next navigation. */
  private activeChain: RouteChainEntry[] = [];

  /** The component tag of the currently active leaf route. */
  get activeComponent(): string {
    const leaf = this.activeChain[this.activeChain.length - 1];
    return leaf?.component ?? '';
  }

  /** The bound params of the currently active leaf route. */
  get activeParams(): Record<string, string> {
    const leaf = this.activeChain[this.activeChain.length - 1];
    return leaf?.params ?? {};
  }

  /** Start the router. Lazily registers the `<webui-route>` custom element. */
  start(config: RouterConfig = {}): void {
    if (this.started) return;
    this.started = true;
    this.config = config;
    this.loaders = config.loaders ?? {};

    if (!customElements.get(ROUTE_SELECTOR)) {
      customElements.define(ROUTE_SELECTOR, WebUIRouteElement);
    }

    this.inventory = document.querySelector('meta[name="webui-inventory"]')?.getAttribute('content') ?? '';

    const nav = window.navigation;
    const handler = (event: NavigateEvent) => {
      if (!event.canIntercept || event.hashChange) return;
      const url = new URL(event.destination.url);
      if (url.origin !== location.origin) return;
      event.intercept({
        handler: async () => {
          const navigate = async (): Promise<void> => {
            try {
              await this.handleNavigation(buildNavigationTarget(url, this.config.basePath ?? ''));
            } catch (err) {
              console.error('[Router] Navigation error:', err);
            }
          };
          if (document.startViewTransition) {
            await document.startViewTransition(navigate).finished;
          } else {
            await navigate();
          }
        },
      });
    };
    nav.addEventListener('navigate', handler);
    this.cleanupFns.push(() => nav.removeEventListener('navigate', handler));

    this.handleNavigation(this.currentTarget());
  }

  /** Navigate to a new path. */
  navigate(path: string): void {
    const fullPath = prependBasePath(path, this.config.basePath ?? '');
    window.navigation.navigate(fullPath);
  }

  /** Navigate back. */
  back(): void {
    window.navigation.back();
  }

  /**
   * Release cached templates to free memory. Removes entries from
   * `window.__webui_templates` and clears their inventory bits so the
   * server will re-send them on the next navigation that needs them.
   *
   * The framework's `templateCache` is a `WeakMap` keyed by the same
   * meta objects, so those entries become GC-eligible automatically.
   *
   * @param tags - Component tag names to release (e.g. `['section-page']`).
   *               Omit to release all non-active templates.
   */
  releaseTemplates(tags?: string[]): void {
    const registry = window.__webui_templates;
    if (!registry) return;

    const activeSet = new Set(this.activeChain.map(e => e.component));
    const toRelease = tags
      ? tags.filter(t => !activeSet.has(t))
      : Object.keys(registry).filter(t => !activeSet.has(t));

    if (toRelease.length === 0) return;

    // Parse inventory hex → bytes, clear bits, re-encode
    const inv = parseInventoryHex(this.inventory);
    for (const tag of toRelease) {
      delete registry[tag];
      clearInventoryBit(inv, tag);
    }
    this.inventory = encodeInventoryHex(inv);
  }
  /** Tear down. */
  destroy(): void {
    this.loaderPromises.clear();
    this.loaders = {};
    this.activeChain = [];
    for (const fn of this.cleanupFns) fn();
    this.cleanupFns = [];
    this.started = false;
  }

  // ── Route matching ──────────────────────────────────────────────

  /**
   * Core navigation handler — called on initial load and every client-side navigation.
   *
   * On initial load: builds the active chain from SSR'd `<webui-route active>` elements.
   * On subsequent navigations: fetches a JSON partial from the server (which includes
   * the matched route chain), diffs against the current chain to find the first changed
   * level, deactivates old routes from the leaf up, and mounts new components from the
   * change level down. Parent components above the change level are preserved and
   * receive fresh state from the server.
   */
  private async handleNavigation(target: NavigationTarget): Promise<void> {
    const { requestPath } = target;

    if (this.isInitialNavigation) {
      this.activeChain = this.buildChainFromSSR();
      for (const entry of this.activeChain) {
        if (entry.component) await this.ensureComponentLoaded(entry.component);
      }
      if (this.config.dev) {
        this.validateRoutes();
      }
      this.isInitialNavigation = false;
    } else {
      const partialData = await this.fetchPartial(requestPath);
      if (!partialData) return;

      const newChain: RouteChainEntry[] = (partialData.chain ?? []).map(e => ({
        component: e.component ?? '',
        path: e.path ?? '',
        params: e.params ?? {},
        exact: e.exact,
      }));

      if (newChain.length === 0) {
        console.warn(`[Router] No route matched for path: ${requestPath}`);
        window.location.href = prependBasePath(requestPath, this.config.basePath ?? '');
        return;
      }

      const changeLevel = this.findChangeLevel(this.activeChain, newChain);

      // When only query params change (same route, different ?sort= etc.),
      // changeLevel equals chain length so nothing remounts. Detect this
      // and re-apply state to all components in the chain from the server's
      // fresh partial response.
      const isQueryOnlyChange = changeLevel === newChain.length && newChain.length > 0;

      // Deactivate old chain from leaf up
      for (let i = this.activeChain.length - 1; i >= changeLevel; i--) {
        if (this.activeChain[i].el) {
          deactivateRoute(this.activeChain[i].el!);
        }
      }

      // Transfer DOM elements to retained levels
      for (let i = 0; i < changeLevel; i++) {
        newChain[i].el = this.activeChain[i].el;
      }

      // Re-apply state to retained (non-remounted) parent components.
      if (changeLevel > 0 || isQueryOnlyChange) {
        const end = isQueryOnlyChange ? newChain.length : changeLevel;
        for (let i = 0; i < end; i++) {
          this.applyState(newChain[i], partialData);
        }
      }

      // Mount from the change level down — everything above is preserved.
      // Each level finds or creates its <webui-route> element, loads the
      // component JS if lazy, and mounts the component into the route.
      for (let i = changeLevel; i < newChain.length; i++) {
        const entry = newChain[i];
        const parent = i > 0 ? newChain[i - 1] : null;
        const routeEl = this.findOrCreateRouteElement(parent, entry);
        entry.el = routeEl;

        if (entry.component && partialData) {
          await this.ensureComponentLoaded(entry.component);
          await this.mountComponent(routeEl, entry.component, partialData, entry.params);
        }

        activateRoute(routeEl, entry.params);
      }

      this.activeChain = newChain;
    }

    const leaf = this.activeChain[this.activeChain.length - 1];
    const detail: NavigationEvent = {
      component: leaf?.component ?? '',
      params: leaf?.params ?? {},
      path: requestPath,
    };
    window.dispatchEvent(new CustomEvent('webui:route:navigated', { detail }));
  }

  /**
   * Find or create a `<webui-route>` DOM element for a chain entry.
   * For top-level routes, searches direct children of `<body>`.
   * For nested routes, searches the parent component's shadow DOM.
   * Creates the element if not found.
   */
  private findOrCreateRouteElement(
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

    // For nested routes, search in parent component's shadow root
    if (parent.el) {
      const compEl = parent.el.querySelector(parent.component);
      if (compEl?.shadowRoot) {
        for (const child of compEl.shadowRoot.querySelectorAll(ROUTE_SELECTOR)) {
          if (child.getAttribute('component') === entry.component) {
            return child as HTMLElement;
          }
        }

        // Not found — create in the outlet area of parent's shadow root
        const stub = createRouteStub(entry);
        const outletMarker = compEl.shadowRoot.querySelector('outlet');
        if (outletMarker?.parentElement) {
          outletMarker.parentElement.insertBefore(stub, outletMarker.nextSibling);
        } else {
          compEl.shadowRoot.appendChild(stub);
        }
        return stub;
      }
    }

    // Fallback: create and append to body
    const el = createRouteStub(entry);
    document.body.appendChild(el);
    return el;
  }

  /**
   * Build the initial route chain from SSR'd active routes in the DOM.
   * Walks down through active `<webui-route>` elements.
   * Uses the native URLPattern API (Chromium 95+) to extract params.
   */
  private buildChainFromSSR(): RouteChainEntry[] {
    const chain: RouteChainEntry[] = [];
    let currentRoutes = this.discoverTopRoutes();
    let currentBase = '/';

    while (currentRoutes.length > 0) {
      const activeEl = currentRoutes.find(el => el.hasAttribute('active'));
      if (!activeEl) break;

      const rawPath = routePath(activeEl);
      const comp = routeComponent(activeEl);
      const pathname = window.location.pathname;

      // Resolve relative path against current base
      const resolvedPath = this.resolveRoutePath(rawPath, currentBase);

      // Use URLPattern to extract params
      const params = this.extractParams(resolvedPath, pathname);

      // Compute new base from the resolved pattern (consumed segments)
      currentBase = this.computeRouteBase(resolvedPath, pathname);

      chain.push({
        component: comp,
        path: rawPath,
        params,
        exact: isExact(activeEl),
        el: activeEl,
      });
      activateRoute(activeEl, params);

      currentRoutes = this.discoverChildRoutes(activeEl);
    }

    return chain;
  }

  /** Resolve a relative route path against a base. */
  private resolveRoutePath(path: string, base: string): string {
    if (path.length === 0) return base;
    if (path.startsWith('/')) return path;
    const relative = path.startsWith('./') ? path.slice(2) : path;
    if (relative.length === 0) return base;
    const b = base.endsWith('/') ? base : `${base}/`;
    return `${b}${relative}`;
  }

  /** Extract params from a pathname using URLPattern. */
  private extractParams(pattern: string, pathname: string): Record<string, string> {
    try {
      const urlPattern = new URLPattern({ pathname: pattern });
      const result = urlPattern.exec({ pathname });
      if (!result) return {};
      const groups = result.pathname.groups;
      const params: Record<string, string> = {};
      for (const [k, v] of Object.entries(groups)) {
        if (v !== undefined) params[k] = v;
      }
      return params;
    } catch {
      return {};
    }
  }

  /** Compute the route base from the matched portion of the pathname. */
  private computeRouteBase(pattern: string, pathname: string): string {
    // Count non-param segments in the pattern to determine how many
    // pathname segments this route consumes
    const patternParts = pattern.split('/').filter(Boolean);
    const pathParts = pathname.split('/').filter(Boolean);
    const consumed = Math.min(patternParts.length, pathParts.length);
    if (consumed === 0) return '/';
    return '/' + pathParts.slice(0, consumed).join('/');
  }

  /**
   * Compare old and new chains to find the first level that differs.
   * Returns the index of the first changed level.
   */
  private findChangeLevel(oldChain: RouteChainEntry[], newChain: RouteChainEntry[]): number {
    const len = Math.min(oldChain.length, newChain.length);
    for (let i = 0; i < len; i++) {
      if (
        oldChain[i].component !== newChain[i].component ||
        !this.paramsEqual(oldChain[i].params, newChain[i].params)
      ) {
        return i;
      }
    }
    // If chains differ in length, change starts at the shorter length
    return len;
  }

  private paramsEqual(a: Record<string, string>, b: Record<string, string>): boolean {
    const keysA = Object.keys(a);
    const keysB = Object.keys(b);
    if (keysA.length !== keysB.length) return false;
    for (const key of keysA) {
      if (a[key] !== b[key]) return false;
    }
    return true;
  }

  // ── Fetch + Mount ──────────────────────────────────────────────

  private async fetchPartial(requestPath: string): Promise<(PartialResponse & { inventory?: string }) | null> {
    const fullPath = prependBasePath(requestPath, this.config.basePath ?? '');
    const headers: Record<string, string> = { 'Accept': 'application/json' };
    if (this.inventory) headers['X-WebUI-Inventory'] = this.inventory;
    const resp = await fetch(fullPath, { headers });

    if (!resp.ok) return null;

    const data = await resp.json() as PartialResponse & { inventory?: string };

    if (data.inventory) {
      this.updateInventory(data.inventory);
    }

    // Register any new templates using a DocumentFragment (single append)
    if (data.templates.length > 0) {
      const frag = document.createDocumentFragment();
      const container = document.createElement('div');
      for (const tmpl of data.templates) {
        container.innerHTML = tmpl;
        while (container.firstChild) {
          frag.appendChild(container.firstChild);
        }
      }
      document.body.appendChild(frag);
    }

    return data;
  }

  private async mountComponent(
    routeEl: HTMLElement,
    componentTag: string,
    data: PartialResponse,
    params: Record<string, string>,
  ): Promise<void> {
    const component = document.createElement(componentTag);
    routeEl.textContent = '';
    routeEl.appendChild(component);

    await this.ensureComponentLoaded(componentTag);
    await customElements.whenDefined(componentTag);

    // Wait for the component's template to render (defer-and-hydrate
    // components render asynchronously via prepare()). The shadow root
    // must be populated before we can find <outlet> markers or call
    // setInitialState on children that live in the shadow DOM.
    await this.waitForShadowReady(component);

    if (typeof (component as any).setInitialState === 'function') {
      // Component defines custom state handler — use it
      (component as any).setInitialState(data.state, params);
    } else {
      // Auto-set: apply state as attributes (mirrors SSR emit_state_attributes)
      this.applyStateAsAttributes(component, data.state, params);
    }
  }

  /**
   * Apply state to a mounted component — uses `setInitialState` if defined,
   * otherwise falls back to setting attributes (mirrors SSR behavior).
   */
  private applyState(entry: RouteChainEntry, data: PartialResponse): void {
    if (!entry.component || !entry.el) return;
    const compEl = entry.el.querySelector(entry.component) as any;
    if (!compEl) return;
    if (typeof compEl.setInitialState === 'function') {
      compEl.setInitialState(data.state, entry.params);
    } else {
      this.applyStateAsAttributes(compEl, data.state, entry.params);
    }
  }

  /**
   * Apply state values as HTML attributes on a component element.
   *
   * Mirrors the server's `emit_state_attributes` behavior: scalar values
   * (string, number, boolean) become individual attributes in kebab-case.
   * Objects and arrays are serialized as a `data-state` JSON attribute.
   * Route params are also set as attributes.
   *
   * This is the zero-code default — components using FAST-HTML `@attr`
   * bindings receive state automatically without implementing `setInitialState`.
   */
  private applyStateAsAttributes(
    el: HTMLElement,
    state: Record<string, unknown>,
    params: Record<string, string>,
  ): void {
    const toKebab = (k: string): string => k.replace(/[A-Z]/g, m => `-${m.toLowerCase()}`);
    const complex: Record<string, unknown> = {};

    for (const [key, value] of Object.entries(state)) {
      if (value == null) continue;
      if (typeof value === 'object') {
        complex[key] = value;
      } else {
        el.setAttribute(toKebab(key), String(value));
      }
    }

    if (Object.keys(complex).length > 0) {
      el.setAttribute('data-state', JSON.stringify(complex));
    }

    for (const [key, value] of Object.entries(params)) {
      el.setAttribute(toKebab(key), value);
    }
  }

  /**
   * Wait for an element's shadow root to be populated with template content.
   * defer-and-hydrate components render their template asynchronously after
   * connectedCallback. After `whenDefined` resolves, a single animation frame
   * is sufficient for the shadow root to populate.
   */
  private waitForShadowReady(el: HTMLElement): Promise<void> {
    if (el.shadowRoot && el.shadowRoot.children.length > 0) {
      return Promise.resolve();
    }
    return new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
  }

  // ── Lazy Loading ────────────────────────────────────────────────

  /**
   * Ensure a component's JS module is loaded. If a lazy loader is
   * configured for this tag and the element isn't already registered,
   * invoke the loader. The promise is cached so each loader runs at
   * most once.
   */
  private async ensureComponentLoaded(tag: string): Promise<void> {
    if (customElements.get(tag)) return;

    const loader = this.loaders[tag];
    if (!loader) return;

    let promise = this.loaderPromises.get(tag);
    if (!promise) {
      promise = loader().then(() => {});
      this.loaderPromises.set(tag, promise);
    }
    await promise;
  }

  // ── Dev-mode Validation ─────────────────────────────────────────

  /**
   * Development-mode validation of the route configuration.
   * Warns about common mistakes via console.warn.
   */
  private validateRoutes(): void {
    for (const { el } of this.activeChain) {
      if (!el) continue;
      const comp = routeComponent(el);
      if (!comp) continue;
      const sr = el.querySelector(comp)?.shadowRoot;
      if (!sr) continue;

      const hasOutlet = sr.querySelector('outlet') !== null;
      const hasChildren = sr.querySelector(ROUTE_SELECTOR) !== null;
      const path = routePath(el);

      if (!hasChildren && !hasOutlet && !isExact(el) && path !== '/') {
        console.warn(
          `[Router Dev] Route "${path}" (${comp}) is a leaf route without "exact". ` +
          `Add "exact" to prevent unintended prefix matching.`,
        );
      }

      if (hasOutlet && isExact(el)) {
        console.warn(
          `[Router Dev] Route "${path}" (${comp}) has <outlet /> with "exact". ` +
          `Remove "exact" — child routes will never match.`,
        );
      }
    }
  }

  // ── Discovery ───────────────────────────────────────────────────

  /**
   * Find top-level route elements — direct children of `<body>` or
   * the document (not nested inside another route's outlet).
   */
  private discoverTopRoutes(): HTMLElement[] {
    const results: HTMLElement[] = [];
    // Top-level routes are direct light DOM children of the body
    for (const child of document.body.children) {
      if (child.tagName === 'WEBUI-ROUTE') {
        results.push(child as HTMLElement);
      }
    }
    return results;
  }

  /**
   * Find child route elements inside a parent route's component shadow DOM.
   * Traverses: parent route → component → component's shadow DOM → <webui-route> elements.
   */
  private discoverChildRoutes(parentRoute: HTMLElement): HTMLElement[] {
    const results: HTMLElement[] = [];
    const comp = routeComponent(parentRoute);
    if (!comp) return results;

    const compEl = parentRoute.querySelector(comp);
    if (!compEl?.shadowRoot) return results;

    // Child <webui-route> elements are directly in the component's shadow root
    for (const child of compEl.shadowRoot.querySelectorAll(ROUTE_SELECTOR)) {
      results.push(child as HTMLElement);
    }

    return results;
  }

  private currentTarget(): NavigationTarget {
    return buildNavigationTarget(new URL(window.location.href), this.config.basePath ?? '');
  }

  // ── Component Inventory ────────────────────────────────────────

  private updateInventory(serverInventory: string): void {
    this.inventory = serverInventory;
  }
}

/** Singleton router instance. */
export const Router = new WebUIRouter();
